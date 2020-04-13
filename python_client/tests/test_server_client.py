"""
Integration test of basic server functionality and validation that the `Client` uses the correct
Http routes.
"""

from datetime import timedelta
from threading import Thread
from time import sleep

import pytest  # type: ignore

from . import throttle_client


def test_non_blocking_acquire():
    """
    Verify the results of a non-blocking request to acquire.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        first = client.new_peer()
        second = client.new_peer()
        # Acquire first lease
        client.acquire(first, "A")
        # Second lease is pending, because we still hold first
        assert not client.acquire(second, "A")
        # A request to the server is also telling us so
        assert not client.is_acquired(second)
        client.release(first)
        # After releasing the first lease the second lease should no longer be
        # pending
        assert client.is_acquired(second)


def test_removal_of_expired_leases():
    """Verify the removal of expired leases."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        peer = client.new_peer()
        client.acquire(peer, "A", expires_in=timedelta(seconds=1))
        sleep(2)
        client.remove_expired()
        assert client.remainder("A") == 1  # Semaphore should be free again


def test_lock_blocks():
    """
    acquire must blocks until the semaphore count allows for the lock to be acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        first = client.new_peer()
        second = client.new_peer()
        # Acquire first lock
        client.acquire(first, "A")

        acquired = False

        def wait_for_second_lock():
            nonlocal acquired
            acquired = client.acquire(second, semaphore="A", block_for=timedelta(seconds=2))

        t = Thread(target=wait_for_second_lock)
        t.start()
        client.release(first)
        t.join()
        # Second lock is no longer pending, because we released first and t is finished
        assert acquired


def test_pending_leases_dont_expire():
    """
    Test that leases do not expire, while they wait for pending admissions.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        blocker = client.new_peer()
        # Acquire "A", so subsequent locks are pending
        client.acquire(blocker, "A")

        peer = client.new_peer(expires_in=timedelta(seconds=1))
        # This lease should be pending
        client.acquire(peer, semaphore="A", block_for=timedelta(seconds=1.5))
        # The initial timeout of one second should have been expired by now,
        # yet nothing is removed
        assert client.remove_expired() == 0


def test_restore_peer_with_unknown_semaphore():
    """
    A revenant Peer may not acquire a semaphore which does not exist on the server.
    """
    # Restart Server without "A"
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match="Unknown semaphore"):
            # Bogus peer id, presumably from a previous run, before lock losts its state
            client.restore(peer_id=5, acquired={"A": 1})


def test_does_not_starve_large_locks():
    """This tests verifies that large locks do not get starved by many smaller ones"""
    with throttle_client(b"[semaphores]\nA=5") as client:
        small = client.new_peer()
        big = client.new_peer()
        other_small = client.new_peer()
        # This lock is acquired immediatly decrementing the semaphore count to 4
        assert client.acquire(small, "A", count=1)
        # Now try a large one. Of course we can not acquire it yet
        assert not client.acquire(big, "A", count=5)
        # This one could be acquired due to semaphore count, but won't, since the larger one is
        # still pending.
        assert not client.acquire(other_small, "A", count=1)
        # Remainder is still 4
        assert client.remainder("A") == 4

        # We free the first small lock, now the big one can be acquired
        client.release(small)
        assert client.remainder("A") == 0


def test_acquire_three_leases():
    """
    This test verifies that three leases can be acquired at once if the semaphore count
    is three.
    """
    with throttle_client(b"[semaphores]\nA=3") as client:
        p = [client.new_peer() for _ in range(0, 4)]
        assert client.acquire(p[0], "A")
        assert client.acquire(p[1], "A")
        assert client.acquire(p[2], "A")
        assert client.remainder("A") == 0
        assert not client.acquire(p[3], "A")


def test_unblock_immediatly_after_release():
    """
    `acquire` must return immediatly after the pending lock can be acquired and not wait for the
    next request to go through.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        one = client.new_peer()
        two = client.new_peer()
        # Acquire first lease
        client.acquire(one, "A")

        # Wait for `two` in a seperate thread so we can use this thread to release `one`
        def wait_for_two():
            client.acquire(two, "A", block_for=timedelta(seconds=15))

        t = Thread(target=wait_for_two)
        t.start()

        # Unblock `t`
        client.release(one)

        # Three seconds should be ample time for `t` to return
        t.join(3)
        # If `t` is alive, the join timed out, which should not be the case
        assert not t.is_alive()


def test_server_timeout():
    """
    Server should answer request to `acquire` if the timeout has elapsed.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        one = client.new_peer()
        two = client.new_peer()
        # Acquire first lease
        _ = client.acquire(one, "A")

        # Wait for two in a seperate thread so we do not block forever if this test
        # fails.
        def wait_for_two():
            client.acquire(two, "A", block_for=timedelta(seconds=0.5))

        t = Thread(target=wait_for_two)
        t.start()

        # Three seconds should be ample time for `t` to return
        t.join(3)
        # If `t` is alive, the join timed out, which should not be the case
        assert not t.is_alive()


def test_resolve_leases_immediatly_after_expiration():
    """
    `acquire` must return immediatly after the pending lock can be acquired and not wait for the
    next request to go through. In this test the peer previously holding the lock expires rather
    than being deleted explicitly.
    """
    with throttle_client(
        b'litter_collection_interval = "10ms"\n' b"[semaphores]\nA=1"
    ) as client:
        one = client.new_peer()
        two = client.new_peer()

        # Acquire first lease
        client.acquire(one, "A")

        # Wait for it in a seperate thread so we can use this thread to release `one`
        def wait_for_two():
            client.acquire(two, "A", block_for=timedelta(seconds=20))

        t = Thread(target=wait_for_two)
        t.start()

        # Unblock `t`. With expiration
        client.expiration_time = timedelta(milliseconds=500)
        client.heartbeat(one)

        # Three seconds should be ample time for `t` to return
        t.join(4)
        # If `t` is alive, the join timed out, which should not be the case
        assert not t.is_alive()


def test_acquire():
    """
    `acquire` must return `False` while pending and `True` once lock is acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        one = client.new_peer()
        two = client.new_peer()
        # Acquire first lease
        client.acquire(one, "A")
        # Second is pending
        assert not client.acquire(two, "A", block_for=timedelta(milliseconds=10))
        # Release one, so second is acquired
        client.release(one)
        assert client.acquire(two, "A")
