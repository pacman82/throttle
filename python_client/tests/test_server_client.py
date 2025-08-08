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
        first = client.new_peer(expires_in=timedelta(minutes=1))
        second = client.new_peer(expires_in=timedelta(minutes=1))
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


def test_locks_of_expired_peers_are_released():
    """If a peer is removed due to expiration, it's locks must be released."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        peer = client.new_peer(expires_in=timedelta(seconds=3))
        client.acquire(peer, "A", expires_in=timedelta(seconds=0))
        assert client.remainder("A") == 1  # Semaphore should be free again


def test_lock_blocks():
    """
    Acquire must block until the semaphore count allows for the lock to be acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        first = client.new_peer(expires_in=timedelta(minutes=1))
        second = client.new_peer(expires_in=timedelta(minutes=1))
        # Acquire first lock
        client.acquire(first, "A")

        acquired = False

        def wait_for_second_lock():
            nonlocal acquired
            acquired = client.acquire(
                second, semaphore="A", block_for=timedelta(seconds=2)
            )

        t = Thread(target=wait_for_second_lock)
        t.start()
        client.release(first)
        t.join()
        # Second lock is no longer pending, because we released first and t is finished
        assert acquired


def test_acquire_can_prolong_lifetime_of_peer():
    """
    Test that peers do not expire, while they wait for pending locks.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        blocker = client.new_peer(expires_in=timedelta(minutes=1))
        # Acquire "A", so subsequent locks are pending
        client.acquire(blocker, "A")

        peer_id = client.new_peer(expires_in=timedelta(seconds=3))
        # This lock can not be acquired due to `blocker` holding the lock. This request
        # is going to block for one second. After which the peer should have been
        # expired. Yet acquire can prolong the lifetime of the peer.
        client.acquire(
            peer_id,
            semaphore="A",
            block_for=timedelta(seconds=3),
            expires_in=timedelta(seconds=100),
        )
        # The initial timeout of one second should have been expired by now, yet peer must still be
        # alive. We validate this calling a method for the peer and observing that "unknown peer"
        # is **not** raised
        assert not client.is_acquired(peer_id=peer_id)


def test_restore_peer_with_unknown_semaphore():
    """
    A revenant Peer may not acquire a semaphore which does not exist on the server.
    """
    # Restart Server without "A"
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match="Unknown semaphore"):
            # Bogus peer id, presumably from a previous run, before lock losts its state
            client.restore(
                peer_id=5, acquired={"A": 1}, expires_in=timedelta(minutes=1)
            )


def test_does_not_starve_large_locks():
    """This tests verifies that large locks do not get starved by many smaller ones"""
    with throttle_client(b"[semaphores]\nA=5") as client:
        small = client.new_peer(expires_in=timedelta(minutes=1))
        big = client.new_peer(expires_in=timedelta(minutes=1))
        other_small = client.new_peer(expires_in=timedelta(minutes=1))
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
        p = [client.new_peer(expires_in=timedelta(minutes=1)) for _ in range(0, 4)]
        assert client.acquire(p[0], "A")
        assert client.acquire(p[1], "A")
        assert client.acquire(p[2], "A")
        assert client.remainder("A") == 0
        assert not client.acquire(p[3], "A")


def test_unblock_immediatly_after_release():
    """
    `acquire` must return immediatly after the pending lock can be acquired and not wait
    for the next request to go through.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        one = client.new_peer(expires_in=timedelta(minutes=1))
        two = client.new_peer(expires_in=timedelta(minutes=1))
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
        one = client.new_peer(expires_in=timedelta(minutes=1))
        two = client.new_peer(expires_in=timedelta(minutes=1))
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


def test_acquire_locks_immediatly_after_expiration():
    """
    `acquire` must return immediatly after the pending lock can be acquired and not wait for the
    next request to go through. In this test the peer previously holding the lock expires rather
    than being deleted explicitly.
    """
    with throttle_client(
        b'litter_collection_interval = "10ms"\n[semaphores]\nA=1'
    ) as client:
        one = client.new_peer(expires_in=timedelta(minutes=1))
        two = client.new_peer(expires_in=timedelta(minutes=1))

        # Acquire first lease
        client.acquire(one, "A")

        # Wait for it in a seperate thread so we can use this thread to release `one`
        def wait_for_two():
            client.acquire(two, "A", block_for=timedelta(seconds=20))

        t = Thread(target=wait_for_two)
        t.start()

        # Unblock `t`. With expiration
        client.heartbeat(one, expires_in=timedelta(seconds=0))

        # Three seconds should be ample time for `t` to return
        t.join(4)
        # If `t` is alive, the join timed out, which should not be the case
        assert not t.is_alive()


def test_acquire():
    """
    `acquire` must return `False` while pending and `True` once lock is acquired.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        one = client.new_peer(expires_in=timedelta(minutes=1))
        two = client.new_peer(expires_in=timedelta(minutes=1))
        # Acquire first lease
        client.acquire(one, "A")
        # Second is pending
        assert not client.acquire(two, "A", block_for=timedelta(milliseconds=10))
        # Release one, so second is acquired
        client.release(one)
        assert client.acquire(two, "A")


def test_acquire_two_locks_with_one_peer():
    """
    Releasing a lock, while keeping its peer, must still enable other locks to be
    acquired.
    """
    with throttle_client(
        b"[semaphores.A]\nmax=1\nlevel=1\n[semaphores.B]\nmax=1\nlevel=0\n"
    ) as client:
        one = client.new_peer(expires_in=timedelta(minutes=1))
        two = client.new_peer(expires_in=timedelta(minutes=1))

        # Acquire two leases with same peer
        assert client.acquire(one, "A")
        assert client.acquire(one, "B")

        assert not client.acquire(two, "B")

        # Release one "B", so two is acquired
        client.release_lock(one, "B")

        assert client.is_acquired(two)
        # First peer is still active and holds a lock
        assert client.is_acquired(one)


def test_litter_collection():
    """
    Verify that leases don't leak thanks to litter collection
    """
    with throttle_client(
        (b'litter_collection_interval="10ms"\n[semaphores]\nA=1\n')
    ) as client:
        # Acquire lease, but since we don't use the context manager we never release
        # it.
        peer = client.new_peer(expires_in=timedelta(minutes=1))
        _ = client.acquire(peer, "A", expires_in=timedelta(seconds=0.1))
        # No, worry time will take care of this.
        sleep(0.2)
        assert client.remainder("A") == 1
