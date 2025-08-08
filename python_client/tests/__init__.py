import subprocess
from contextlib import contextmanager
from tempfile import NamedTemporaryFile
from time import sleep

import requests

from throttle_client import Client, clear_peers

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
            except requests.ConnectionError:
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


@contextmanager
def throttle(config: bytes):
    with NamedTemporaryFile(delete=False) as cfg:
        cfg.write(config)
        cfg.close()
        with cargo_main(cfg=cfg.name) as _:
            try:
                yield BASE_URL
            finally:
                clear_peers()
