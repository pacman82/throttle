"""Test the clients `lock` interface."""

from datetime import timedelta
from tempfile import NamedTemporaryFile
from threading import Thread
from time import sleep

import pytest  # type: ignore

from throttle_client import (
    Client,
    Peer,
    PeerWithHeartbeat,
    Timeout,
    get_local_peer,
    lock,
    set_local_peer,
)

from . import BASE_URL, cargo_main, throttle


def test_local_peer():
    """
    Must create peer with heartbeat
    """
    with throttle(b"[semaphores]") as url:
        peer = get_local_peer(url)
        assert isinstance(peer, PeerWithHeartbeat)


def test_error_on_leasing_unknown_semaphore():
    """
    Verify what error we receive than acquiring a semaphore for a ressource
    unknown to the server.
    """
    with throttle(b"[semaphores]") as url:
        with pytest.raises(Exception, match=r"Unknown semaphore"):
            with lock(url, "Unknown"):
                pass


def test_remainder():
    """
    Verify that we can acquire a lock to semaphore and release it
    """
    with throttle(b"[semaphores]\nA=1") as url:
        client = Client(url)
        assert 1 == client.remainder("A")
        with lock(url, "A"):
            assert 0 == client.remainder("A")
        assert 1 == client.remainder("A")


def test_server_recovers_pending_lock_after_state_loss():
    """
    Verify pending leases recover from server state loss and are acquired after reboot.
    """
    acquired_lease = False

    def acquire_lease_concurrent(client):
        with lock(BASE_URL, "A", timeout=timedelta(seconds=10)):
            nonlocal acquired_lease
            acquired_lease = True

    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(b"[semaphores]\nA=1")
        cfg.close()
        client = Client(BASE_URL)
        with cargo_main(cfg=cfg.name) as proc:
            first = Peer(client=client, expiration_time=timedelta(minutes=1))
            # Acquire first peer
            first.acquire("A")
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
    Validates that a heartbeat keeps the lease alive beyond the initial expiration time.
    """
    with throttle(b"[semaphores]\nA=1") as url:
        client = Client(url)
        peer = PeerWithHeartbeat(
            client=client,
            heartbeat_interval=timedelta(seconds=1),
            expiration_time=timedelta(seconds=4),
        )
        set_local_peer(url, peer)
        with lock(url, "A") as _:
            sleep(5)
            # Evens though enough time has passed, our lease should not be
            # expired, thanks to the heartbeat.
            assert client.is_acquired(peer.id)


def test_lock_count_larger_one():
    """
    Assert that locks with a count > 1, decrement the semaphore count accordingly
    """

    with throttle(b"[semaphores]\nA=5") as url:
        client = Client(url)
        with lock(url, "A", count=3):
            assert client.remainder("A") == 2
        assert client.remainder("A") == 5


def test_exception():
    """
    Assert that lock is freed in the presence of exceptions in the client code
    """

    class OutOfCheese(Exception):
        pass

    checked = False

    with throttle(b"[semaphores]\nA=1") as url:
        client = Client(url)
        try:
            with lock(url, "A"):
                raise OutOfCheese()
        except OutOfCheese:
            assert client.remainder("A") == 1
            checked = True

    # Make sure we entered exception handling
    assert checked


def test_try_lock():
    """
    Assert that a call to lock raises Timout Exception if pending to long
    """
    with throttle(b"[semaphores]\nA=1") as url:
        # We hold the lease, all following calls are going to block
        first = Peer.from_server_url(url)
        first.acquire("A")
        with pytest.raises(Timeout):
            with lock(BASE_URL, "A", timeout=timedelta(seconds=1)):
                pass


def test_recover_from_unknown_peer_during_acquisition_lock():
    """
    The lock interface must recreate the peer if it is removed from the server, between
    `new_peer` and acquire.
    """
    acquired_lease = False

    def acquire_lease_concurrent(client: Client):
        # Simulate expired peer: Create peer with bogus id.
        peer = PeerWithHeartbeat(client=client, id=42)
        set_local_peer(BASE_URL, peer)
        # Lock must now restore the peer, after hitting `Unknown Peer` and acquire the
        # lock to A afterwards.
        with lock(BASE_URL, "A", timeout=timedelta(seconds=10)):
            nonlocal acquired_lease
            acquired_lease = True

    # Trigger `UnknownPeer` using litter collection. Let the peer expire really fast
    with throttle(b"[semaphores]\nA=1") as url:
        client = Client(url)
        t = Thread(target=acquire_lease_concurrent, args=[client])
        t.start()
        t.join(timeout=10)

        assert acquired_lease


def test_nested_locks():
    """
    Nested locks should be well behaved
    """
    with throttle(b"[semaphores]\nA={ max=1, level=1 }\nB={ max=1, level=0 }") as url:
        client = Client(url)
        with lock(url, "A"):
            assert client.remainder("A") == 0
            assert client.remainder("B") == 1

            with lock(url, "B"):
                assert client.remainder("A") == 0
                assert client.remainder("B") == 0

            assert client.remainder("A") == 0
            assert client.remainder("B") == 1

        assert client.remainder("A") == 1
        assert client.remainder("B") == 1


def test_semaphore_named_slash():
    """
    A semaphore named '/&' should behave correctly.
    """
    with throttle(b'[semaphores]\n"/&"=1') as url:
        client = Client(url)
        assert 1 == client.remainder("/&")
        with lock(url, "/&"):
            assert 0 == client.remainder("/&")
        # assert 1 == client.remainder("/")
