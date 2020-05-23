import pickle

from throttle_client import Client


def test_client():
    """Ensure we can pickle client."""

    a = Client("https://dummy-endpoint")
    serialized = pickle.dumps(a)
    b = pickle.loads(serialized)

    assert a.base_url == b.base_url
