from throttle_client import Client, lock, Timeout
from contextlib import contextmanager
from tempfile import NamedTemporaryFile
from threading import Thread
from time import sleep
from datetime import timedelta

import pytest
import requests
import subprocess

# Endpoint of throttle server started by cargo main
BASE_URL = "http://localhost:8000"


@contextmanager
def cargo_main(cfg: str):
    """
    Use as a test fixture to ensure the throttle server is running with all the
    recent code changes.
    """
    # Start for server
    proc = subprocess.Popen(["cargo", "run", "--", "-c", cfg])

    try:
        # Wait for server to boot and answer health request
        no_connection = True
        while proc.poll() is None and no_connection:
            try:
                response = requests.get(f"{BASE_URL}/health", timeout=0.2)
                response.raise_for_status()
                no_connection = False
            except (requests.ConnectionError):
                sleep(0.1)  # Sleep a tenth of a second, don't busy waits

        yield proc
    finally:
        proc.terminate()


@contextmanager
def throttle_client(config: bytes):
    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(config)
        cfg.close()
        with cargo_main(cfg=cfg.name) as _:
            yield Client(BASE_URL)


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


def test_pending_lock():
    """
    Verify the results of a non-blocking request to wait_for_admission
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")
        # Second lease is pending, because we still hold first
        assert second.has_pending()
        # A request to the server is also telling us so
        assert client.wait_for_admission(second, block_for=timedelta(seconds=0))
        client.release(first)
        # After releasing the first lease the second lease should no longer be
        # pending
        assert not client.wait_for_admission(second, block_for=timedelta(seconds=0))


def test_lock_blocks():
    """
    Verify that wait_for_admission blocks until the semaphore count allows for
    the lock to be acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lock
        first = client.acquire("A")
        second = client.acquire("A")

        def wait_for_second_lock():
            client.wait_for_admission(second, block_for=timedelta(seconds=2))

        t = Thread(target=wait_for_second_lock)
        t.start()
        client.release(first)
        t.join()
        # Second lock is no longer pending, because we released first and t is finished
        assert not second.has_pending()


def test_server_recovers_pending_lock_after_state_loss():
    """
    Verify pending leases recover from server state loss and are acquired
    after reboot.
    """

    acquired_lease = False

    def acquire_lease_concurrent():
        with lock(Client("http://localhost:8000"), "A", timeout=timedelta(seconds=5)):
            nonlocal acquired_lease
            acquired_lease = True

    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(b"[semaphores]\nA=1")
        cfg.close()
        client = Client(BASE_URL)
        with cargo_main(cfg=cfg.name) as proc:
            # Acquire first lease
            _ = client.acquire("A")
            # Acquire second lease
            t = Thread(target=acquire_lease_concurrent)
            t.start()
            # Give the acquire request some time to go through, so we hit the edge case
            # of getting an 'Unknown lease' response from the server
            sleep(2)
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


def test_removal_of_expired_leases():
    """Verify the removal of expired leases."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        client.acquire("A", expires_in=timedelta(seconds=1))
        sleep(2)
        client.remove_expired()
        assert client.remainder("A") == 1  # Semaphore should be free again


def test_pending_leases_dont_expire():
    """
    Test that leases do not expire, while they wait for pending admissions.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire once, so subsequent leases are pending
        _ = client.acquire("A")
        # This lease should be pending
        lease = client.acquire("A", expires_in=timedelta(seconds=1))
        client.wait_for_admission(lease, block_for=timedelta(seconds=1.5))
        # The initial timeout of one second should have been expired by now,
        # yet nothing is removed
        assert client.remove_expired() == 0


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


def test_lease_recovery_after_server_reboot():
    """
    Verify that a newly booted server, recovers leases based on heartbeats
    """
    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(b"[semaphores]\nA=1")
        cfg.close()
        client = Client(BASE_URL)
        with cargo_main(cfg.name) as proc:
            with lock(client, "A", heartbeat_interval=timedelta(milliseconds=100)):
                # Kill the server, so it looses all it's in memory state
                proc.kill()
                # And start a new one, with the heartbeat still active
                with cargo_main(cfg.name) as _:
                    # Wait a moment for the heartbeat to update server sate.
                    sleep(0.3)
                    assert client.remainder("A") == 0


def test_litter_collection():
    """
    Verify that leases don't leak thanks to litter collection
    """
    with throttle_client(
        (b'litter_collection_interval="10ms"\n' b"[semaphores]\n" b"A=1\n")
    ) as client:
        client.expiration_time = timedelta(seconds=0.1)
        # Acquire lease, but since we don't use the context manager we never release
        # it.
        _ = client.acquire("A")
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
        l1 = client.acquire("A", count=3)
        l2 = client.acquire("A", count=3)
        assert l2.has_pending()


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
            client.acquire("A", count=2)


def test_try_lock():
    """
    Assert that a call to lock raises Exception returns None after a specified timeout
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # We hold the lease, all following calls are going to block
        _ = client.acquire("A")
        with pytest.raises(Timeout):
            with lock(client, "A", timeout=timedelta(seconds=1)):
                pass
