from throttle_client import Client, lease
from contextlib import contextmanager
from tempfile import NamedTemporaryFile
from threading import Thread
from time import sleep

import pytest
import requests
import subprocess

# Endpoint of throttle server started by cargo main
BASE_URL = "http://localhost:8000"


@contextmanager
def cargo_main(cfg: str):
    """Use as a test fixture to ensure the throttle server is running with all the recent code
       changes.
    """
    # Start for server
    proc = subprocess.Popen(["cargo", "run", "--", "-c", cfg])

    try:
        # Wait for server to boot and answer health request
        no_connection = True
        while proc.poll() == None and no_connection:
            try:
                response = requests.get(f"{BASE_URL}/health", timeout=0.2)
                no_connection = False
            except (requests.ConnectionError):
                sleep(0.1)  # Sleep a tenth of a second, don't busy waits

        response.raise_for_status()

        yield proc
    finally:
        proc.terminate()


@contextmanager
def throttle_client(config: bytes):
    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(config)
        cfg.close()
        with cargo_main(cfg=cfg.name) as _proc:
            yield Client(BASE_URL)


def test_error_on_leasing_unknown_semaphore():
    """Verify what error we receive than acquiring a semaphore for a ressource unknown to the
       server.
    """
    with throttle_client(b"[semaphores]") as client:
        with pytest.raises(Exception, match=r"Unknown semaphore"):
            with lease(client, "Unknown"):
                pass


def test_remainder():
    """Verify that we can lease a semaphore and return the lease to the server
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        assert 1 == client.remainder("A")
        with lease(client, "A"):
            assert 0 == client.remainder("A")
        assert 1 == client.remainder("A")


def test_pending_lease():
    """Verify the results of a non-blocking request to is_pending
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")
        # Second lease is pending, because we still hold first
        assert second.has_pending()
        # A request to the server is also telling us so
        assert client.is_pending(second)
        client.release(first)
        # After releasing the first lease the second lease should no longer be pending
        assert not client.is_pending(second)


def test_blocking_pending_lease():
    """Verify that is_pending blocks until the lease is ready
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")

        def wait_for_second_lease():
            client.is_pending(second, timeout_ms=2000)

        t = Thread(target=wait_for_second_lease)
        t.start()
        client.release(first)
        t.join()
        # Second lease is no longer pending, because we released first and t is finished
        assert not second.has_pending()


def test_server_looses_state_during_pending_lease():
    """Verify pending leases recover from server state loss and are acquired after reboot
    """

    acquired_lease = False

    def acquire_lease_concurrent():
        with lease(Client("http://localhost:8000"), "A") as l:
            nonlocal acquired_lease
            acquired_lease = True

    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(b"[semaphores]\nA=1")
        cfg.close()
        client = Client(BASE_URL)
        with cargo_main(cfg=cfg.name) as proc:
            # Acquire first lease
            first = client.acquire("A")
            # Acquire second lease
            t = Thread(target=acquire_lease_concurrent)
            t.start()
            # Give the acquire request some time to go through, so we hit the edge case of getting
            # an 'Unknown lease' response from the server
            sleep(1)
            proc.kill()

        # Invoking this context manager anew terminates and restarts the server. I.e. it's losing
        # all its state. Note that we started the thread t within the old context and join the
        # pending lease in the new one.
        with cargo_main(cfg=cfg.name):
            # Instead of releasing the first lease, we restarted the server. We don't have a
            # heartbeat for the first lease, so semaphore should be taken by the lease acquired in
            # thread t, if we are able to recover from server reboot during pending leases on the
            # client side.
            t.join(10)
            assert acquired_lease


def test_removal_of_expired_leases():
    """Verify the removal of expired leases."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        client.acquire("A", valid_for_sec=1)
        sleep(2)
        client.remove_expired()
        assert client.remainder("A") == 1  # Semaphore should be free again
