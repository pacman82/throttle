"""Test the clients `lock` interface."""

from datetime import timedelta
from tempfile import NamedTemporaryFile
from threading import Thread
from time import sleep

import pytest  # type: ignore

from throttle_client import Client, Heartbeat, Peer, Timeout, lock

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


def test_multiple_semaphores_remainder():
    """
    Verify that we can acquire and release locks to different semaphores
    """
    with throttle_client(b"[semaphores]\nA=1\nB=1\nC=1") as client:
        with lock(client, "A"):
            assert 0 == client.remainder("A")
            with lock(client, "B"):
                assert 0 == client.remainder("B")
                with lock(client, "C"):
                    assert 0 == client.remainder("C")
        assert 1 == client.remainder("A")
        assert 1 == client.remainder("B")
        assert 1 == client.remainder("C")


def test_server_recovers_pending_lock_after_state_loss():
    """
    Verify pending leases recover from server state loss and are acquired after reboot.
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
            first = client.new_peer()
            # Acquire first peer
            client.acquire(first, "A")
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
        one = client.new_peer()
        two = client.new_peer()
        _ = client.acquire(one, "A", count=3)
        assert not client.acquire(two, "A", count=3)


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
            peer = client.new_peer()
            client.acquire(peer, "A", count=2)


def test_try_lock():
    """
    Assert that a call to lock raises Timout Exception if pending to long
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # We hold the lease, all following calls are going to block
        first = client.new_peer()
        client.acquire(first, "A")
        with pytest.raises(Timeout):
            with lock(client, "A", timeout=timedelta(seconds=1)):
                pass


def test_recover_from_unknown_peer_during_acquisition_lock():
    """
    The lock interface must recreate the peer if it is removed from the server, between
    `new_peer` and acquire.
    """
    acquired_lease = False

    def acquire_lease_concurrent(client):
        with lock(client, "A", timeout=timedelta(seconds=10)):
            nonlocal acquired_lease
            acquired_lease = True

    # Trigger `UnknownPeer` using litter collection. Let the peer expire really fast
    with throttle_client(
        b'litter_collection_interval="10ms"\n' b"[semaphores]\nA=1"
    ) as client:
        # Next peer should expire immediatly
        client.expiration_time = timedelta(seconds=0)
        t = Thread(target=acquire_lease_concurrent, args=[client])
        t.start()

        # Give `new_peer` some time to go through, so we hit the edge case of getting an
        # 'Unknown peer' response from the server during `acquire`.
        sleep(2)

        client.expiration_time = timedelta(minutes=5)
        t.join(timeout=10)

        assert acquired_lease


def test_peer_recovery_after_server_reboot():
    """
    Heartbeat must restore peers, after server reboot.
    """
    peer = Peer(id=5, acquired={"A": 1})
    # Server is shutdown. Boot a new one wich does not know about this peer
    with throttle_client(b"[semaphores]\nA=1") as client:
        heartbeat = Heartbeat(client, peer, interval=timedelta(milliseconds=10))
        heartbeat.start()
        # Wait for heartbeat and restore, to go through
        sleep(2)
        heartbeat.stop()
        # Which implies the remainder of A being 0
        assert client.remainder("A") == 0


def test_nested_locks():
    """
    Nested locks should be well behaved
    """
    with throttle_client(b"[semaphores]\nA=1\nB=1") as client:
        with lock(client, "A") as peer:

            assert client.remainder("A") == 0
            assert client.remainder("B") == 1

            with lock(client, "B", peer=peer):

                assert client.remainder("A") == 0
                assert client.remainder("B") == 0

            assert client.remainder("A") == 0
            assert client.remainder("B") == 1

        assert client.remainder("A") == 1
        assert client.remainder("B") == 1


def test_multiple_peer_recovery_after_server_reboot():
    """
    Heartbeat must restore all locks for a peer, after server reboot.
    """
    # Bogus peer id. Presumably from a peer created before the server reboot.
    peer = Peer(id=42, acquired={"A": 1, "B": 1, "C": 1})

    # Server is shutdown. Boot a new one wich does not know about the peers
    with throttle_client(b"[semaphores]\nA=1\nB=1\nC=1") as client:
        heartbeat = Heartbeat(client, peer, interval=timedelta(milliseconds=10))
        heartbeat.start()
        # Wait for heartbeat and restore, to go through
        sleep(2)
        heartbeat.stop()
        # Which implies the remainder of A, B, C being 0
        assert client.remainder("A") == 0
        assert client.remainder("B") == 0
        assert client.remainder("C") == 0
