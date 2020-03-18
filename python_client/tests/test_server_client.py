"""
Integration test of basic server functionality and validation that the `Client` uses the correct
Http routes.
"""

from datetime import timedelta
from threading import Thread
from time import sleep

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
        assert client.block_until_acquired(second, block_for=timedelta(seconds=0))
        client.release(first)
        # After releasing the first lease the second lease should no longer be
        # pending
        assert not client.block_until_acquired(second, block_for=timedelta(seconds=0))


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
