import pickle

from throttle_client import Client, Peer


def test_client():
    """Ensure we can pickle client."""

    a = Client(f"https://dummy-endpoint")
    serialized = pickle.dumps(a)
    b = pickle.loads(serialized)

    assert a.base_url == b.base_url
