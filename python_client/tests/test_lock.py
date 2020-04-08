"""Test the clients `lock` interface."""

from datetime import timedelta
from tempfile import NamedTemporaryFile
from threading import Thread
from time import sleep

import pytest  # type: ignore

from throttle_client import Client, Timeout, lock

from . import BASE_URL, cargo_main, throttle_client


def test_error_on_leasing_unknown_semaphore():
    """
    Verify what error we receive than acquiring a semaphore for a ressource
    unknown to the server.
    """
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match=r"Unknown semaphore"):
            with lock(client, "Unknown"):
                pass


def test_remainder():
    """
    Verify that we can acquire a lock to semaphore and release it
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        assert 1 == client.remainder("A")
        with lock(client, "A"):
            assert 0 == client.remainder("A")
        assert 1 == client.remainder("A")


def test_server_recovers_pending_lock_after_state_loss():
    """
    Verify pending leases recover from server state loss and are acquired
    after reboot.
    """

    acquired_lease = False

    def acquire_lease_concurrent(client):
        with lock(client, "A", timeout=timedelta(seconds=10)):
            nonlocal acquired_lease
            acquired_lease = True

    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(b"[semaphores]\nA=1")
        cfg.close()
        client = Client(BASE_URL)
        with cargo_main(cfg=cfg.name) as proc:
            # Acquire first peer
            _ = client.acquire_with_new_peer("A")
            # Acquire second lease
            t = Thread(target=acquire_lease_concurrent, args=[client])
            t.start()
            # Give the acquire request some time to go through, so we hit the edge case
            # of getting an 'Unknown peer' response from the server
            sleep(4)
            proc.kill()

        # Invoking this context manager anew terminates and restarts the server. I.e.
        # it's losing all its state. Note that we started the thread t within the old
        # context and join the pending lease in the new one.
        with cargo_main(cfg=cfg.name):
            # Instead of releasing the first lease, we restarted the server. We don't
            # have a heartbeat for the first lease, so semaphore should be taken by the
            # lease acquired in thread t, if we are able to recover from server reboot
            # during pending leases on the client side.
            t.join()
            assert acquired_lease


def test_keep_lease_alive_beyond_expiration():
    """
    Validates that a heartbeat keeps the lease alive beyond the initial
    expiration time.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        client.expiration_time = timedelta(seconds=1)
        with lock(client, "A", heartbeat_interval=timedelta(seconds=0)) as _:
            sleep(1.5)
            # Evens though enough time has passed, our lease should not be
            # expired, thanks to the heartbeat.
            assert client.remove_expired() == 0


def test_litter_collection():
    """
    Verify that leases don't leak thanks to litter collection
    """
    with throttle_client(
        (b'litter_collection_interval="10ms"\n' b"[semaphores]\n" b"A=1\n")
    ) as client:
        # Acquire lease, but since we don't use the context manager we never release
        # it.
        peer = client.new_peer()
        client.expiration_time = timedelta(seconds=0.1)
        _ = client.acquire(peer, "A")
        # No, worry time will take care of this.
        sleep(0.2)
        assert client.remainder("A") == 1


def test_lock_count_larger_one():
    """
    Assert that locks with a count > 1, decrement the semaphore count accordingly
    """

    with throttle_client(b"[semaphores]\nA=5") as client:
        with lock(client, "A", count=3):
            assert client.remainder("A") == 2
        assert client.remainder("A") == 5


def test_lock_count_larger_pends_if_count_is_not_high_enough():
    """
    Assert that we do not overspend an semaphore using lock counts > 1. Even if the
    semaphore count is > 0.
    """

    with throttle_client(b"[semaphores]\nA=5") as client:
        _ = client.acquire_with_new_peer("A", count=3)
        second = client.acquire_with_new_peer("A", count=3)
        assert second.has_pending()


def test_exception():
    """
    Assert that lock is freed in the presence of exceptions in the client code
    """

    with throttle_client(b"[semaphores]\nA=1") as client:
        try:
            with lock("A"):
                raise Exception()
        except Exception:
            assert client.remainder("A") == 1


def test_lock_count_larger_than_full_count():
    """
    Assert that exception is raised, rather than accepting a lock which would pend
    forever.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        with pytest.raises(ValueError, match="block forever"):
            client.acquire_with_new_peer("A", count=2)


def test_try_lock():
    """
    Assert that a call to lock raises Timout Exception if pending to long
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # We hold the lease, all following calls are going to block
        _ = client.acquire_with_new_peer("A")
        with pytest.raises(Timeout):
            with lock(client, "A", timeout=timedelta(seconds=1)):
                pass


def test_put_unknown_semaphore():
    """
    An active revenant of an unknown semaphore should throw an exception.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        a = client.acquire_with_new_peer("A")
    # Restart Server without "A"
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match="Unknown semaphore"):
            client.heartbeat(a)


# Sadly it seems requests doesn't care much for my delete timeout in the release of the lock

# def test_timeout_during_release():
#     """
#     Test ensuring http timeouts during a delete operation are ignored.
#     """

#     # There is room for improvement here. We could fire the request asynchronously and not care
#     # for the response
#     def freeze_server():
#         client.freeze(timedelta(seconds=10))

#     with throttle_client(b"[semaphores]\nA=1") as client:
#         with lock(client, "A", heartbeat_interval=None):
#             t = Thread(target=freeze_server)
#             t.start()
#             # Give freeze request time to pass through
#             sleep(1.0)
#         # If we get to this point without throwing the test is considered successful
#         t.join()
#         assert True
