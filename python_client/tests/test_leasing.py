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
    """Verify the results of a non-blocking request to wait_for_admission
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")
        # Second lease is pending, because we still hold first
        assert second.has_pending()
        # A request to the server is also telling us so
        assert client.wait_for_admission(second)
        client.release(first)
        # After releasing the first lease the second lease should no longer be pending
        assert not client.wait_for_admission(second)


def test_blocking_pending_lease():
    """Verify that wait_for_admission blocks until the lease is ready
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire first lease
        first = client.acquire("A")
        second = client.acquire("A")

        def wait_for_second_lease():
            client.wait_for_admission(second, timeout_ms=2000)

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


def test_pending_leases_dont_expire():
    """Test that leases do not expire, while they wait for pending admissions."""
    with throttle_client(b"[semaphores]\nA=1") as client:
        # Acquire once, so subsequent leases are pending
        _ = client.acquire("A")
        # This lease should be pending
        lease = client.acquire("A", valid_for_sec=1)
        client.wait_for_admission(lease, timeout_ms=1500)
        # The initial timeout of one second should have been expired by now, yet nothing is removed
        assert client.remove_expired() == 0


def test_keep_lease_alive_beyond_expiration():
    """
    Validates that a heartbeat keeps the lease alive beyond the initial expiration time.
    """
    with throttle_client(b"[semaphores]\nA=1") as client:
        client.expiration_time_sec = 2
        with lease(client, "A", heartbeat_interval_sec=1) as l:
            sleep(3)
            # Evens though enough time has passed, our lease should not be expired, thanks to the
            # heartbeat.
            assert client.remove_expired() == 0


# def test_litter_collection():
#     """Verify server releases leases if not kept alive by heartbeat"""
#     with throttle_client(b"[semaphores]\nA=1\n") as client:
#         _ = client.acquire("A", valid_for_sec=1)
