"""
Integration test of basic server functionality and validation that the `Client` uses the correct
Http routes.
"""

from datetime import timedelta
from threading import Thread
from time import sleep

import pytest

from . import throttle_client


def test_pending_lock():
    """
    Verify the results of a non-blocking request to block_until_acquired
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")
        # Second lease is pending, because we still hold first
        assert second.has_pending()
        # A request to the server is also telling us so
        assert not client.is_acquired(second)
        client.release(first)
        # After releasing the first lease the second lease should no longer be
        # pending
        assert client.is_acquired(second)


def test_removal_of_expired_leases():
    """Verify the removal of expired leases."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        client.acquire("A", expires_in=timedelta(seconds=1))
        sleep(2)
        client.remove_expired()
        assert client.remainder("A") == 1  # Semaphore should be free again


def test_lock_blocks():
    """
    Verify that block_until_acquired blocks until the semaphore count allows for
    the lock to be acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lock
        first = client.acquire("A")
        second = client.acquire("A")

        def wait_for_second_lock():
            client.block_until_acquired(second, block_for=timedelta(seconds=2))

        t = Thread(target=wait_for_second_lock)
        t.start()
        client.release(first)
        t.join()
        # Second lock is no longer pending, because we released first and t is finished
        assert not second.has_pending()


def test_pending_leases_dont_expire():
    """
    Test that leases do not expire, while they wait for pending admissions.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire once, so subsequent leases are pending
        _ = client.acquire("A")
        # This lease should be pending
        peer = client.acquire("A", expires_in=timedelta(seconds=1))
        client.block_until_acquired(peer, block_for=timedelta(seconds=1.5))
        # The initial timeout of one second should have been expired by now,
        # yet nothing is removed
        assert client.remove_expired() == 0


def test_block_on_unknown_semaphore():
    """
    A pending revenant of an unknown semaphore should throw an exception.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Only take first, so second one blocks
        _ = client.acquire("A")
        lock_a = client.acquire("A")
    # Restart Server without "A"
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match="Unknown semaphore"):
            client.block_until_acquired(lock_a, block_for=timedelta(seconds=1))


def test_lease_recovery_after_server_reboot():
    """
    Verify that a newly booted server, recovers leases based on heartbeats
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        peer = client.acquire("A")

    # Server is shutdown. Boot a new one wich does not know about this peer
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Heartbeat should restore server state.
        client.heartbeat(peer)
        # Which implies the remainder of A being 0
        assert client.remainder("A") == 0


def test_does_not_starve_large_locks():
    """This tests verifies that large locks do not get starved by many smaller ones"""
    with throttle_client(b"[semaphores]\nA=5") as client:
        # This lock is acquired immediatly decrementing the semaphore count to 4
        lock_small = client.acquire("A")
        # Now try a large one. Of course we can not acquire it yet
        _ = client.acquire("A", count=5)
        # This one could be acquired due to semaphore count, but won't, since the larger one is
        # still pending.
        _ = client.acquire("A")
        # Remainder is still 4
        assert client.remainder("A") == 4

        # We free the first small lock, now the big one can be acquired
        client.release(lock_small)
        assert client.remainder("A") == 0


def test_acquire_three_leases():
    """
    This test verifies that three leases can be acquired at once if the semaphore count
    is three.
    """
    with throttle_client(b"[semaphores]\nA=3") as client:
        assert client.acquire("A").has_active()
        assert client.acquire("A").has_active()
        assert client.acquire("A").has_active()
        assert client.remainder("A") == 0
        assert client.acquire("A").has_pending()


def test_unblock_immediatly_after_release():
    """
    `block_until_acquired` must return immediatly after the pending lock can be acquired and not
    wait for the next request to go through.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        one = client.acquire("A")
        # Second is pending
        two = client.acquire("A")
        # Wait for it in a seperate thread so we can use this thread to release `one`

        def wait_for_two():
            client.block_until_acquired(two, block_for=timedelta(seconds=15))

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
    Server should answer request to `block_until_acquired` if the timeout has elapsed.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        _ = client.acquire("A")
        # Second is pending
        two = client.acquire("A")
        # Wait for it in a seperate thread so we do not block forever if this test
        # fails.

        def wait_for_two():
            client.block_until_acquired(two, block_for=timedelta(seconds=0.5))

        t = Thread(target=wait_for_two)
        t.start()

        # Three seconds should be ample time for `t` to return
        t.join(3)
        # If `t` is alive, the join timed out, which should not be the case
        assert not t.is_alive()
