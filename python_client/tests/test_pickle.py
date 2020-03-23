import pickle

from throttle_client import Client


def test_pickle():
    """Ensure we can pickle client."""

    a = Client(f"https://dummy-endpoint")
    serialized = pickle.dumps(a)
    b = pickle.loads(serialized)

    assert a.base_url == b.base_url
    assert a.expiration_time == b.expiration_time
