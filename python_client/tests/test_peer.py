from datetime import timedelta
from time import sleep

import pytest  # type: ignore

from throttle_client import Peer, PeerWithHeartbeat

from . import throttle_client


def test_lock_count_larger_pends_if_count_is_not_high_enough():
    """
    Assert that we do not overspend an semaphore using lock counts > 1. Even if the
    semaphore count is > 0.
    """
    with throttle_client(b"[semaphores]\nA=5") as client:
        one = Peer(client=client)
        two = Peer(client=client)
        one.acquire("A", count=3)
        assert not two.acquire("A", count=3)


def test_lock_count_larger_than_full_count():
    """
    Should the lock count be larger than the maximum allowed semaphore count an
    exception is raised.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        with pytest.raises(ValueError, match="Lock can never be acquired."):
            peer = Peer(client=client)
            peer.acquire("A", count=2)


def test_peer_recovery_after_server_reboot():
    """
    Heartbeat must restore peers, after server reboot.
    """
    # Server is shutdown. Boot a new one wich does not know about this peer
    with throttle_client(b"[semaphores]\nA=1") as client:
        peer = PeerWithHeartbeat(
            client=client,
            id=5,
            acquired={"A": 1},
            heartbeat_interval=timedelta(milliseconds=10),
        )
        peer.start_heartbeat()
        # Wait for heartbeat and restore, to go through
        sleep(2)
        peer.stop_heartbeat()
        # Which implies the remainder of A being 0
        assert client.remainder("A") == 0


def test_multiple_peer_recovery_after_server_reboot():
    """
    Heartbeat must restore all locks for a peer, after server reboot.
    """
    # Server is shutdown. Boot a new one wich does not know about the peers
    with throttle_client(b"[semaphores]\nA=1\nB=1\nC=1") as client:
        # Bogus peer id. Presumably from a peer created before the server reboot.
        peer = PeerWithHeartbeat(
            client=client,
            id=42,
            acquired={"A": 1, "B": 1, "C": 1},
            heartbeat_interval=timedelta(milliseconds=10),
        )
        peer.start_heartbeat()
        # Wait for heartbeat and restore, to go through
        sleep(2)
        peer.stop_heartbeat()
        # Which implies the remainder of A, B, C being 0
        assert client.remainder("A") == 0
        assert client.remainder("B") == 0
        assert client.remainder("C") == 0
