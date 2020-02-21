import requests
import backoff
import json
from contextlib import contextmanager


class UnknownLease(ValueError):
    """Indicates that the throttle server does not recognize the lease id"""

    def __init__(self):
        super("Unknown lease")


class Lease:
    """A lease to a semaphore from a throttle service."""

    def __init__(self, id: str, pending: bool):
        self.id = id
        self.pending = pending


def _raise_for_status(response: requests.Response):
    """
    Wrapper around Response.raise_for_status, translating 4xx to value errors
    """

    if response.status_code == 400:
        if response.text == "Unknown lease":
            raise UnknownLease
        else:
            # Since this is the client code which made the erroneous request, it is supposedly
            # caused by wrong arguments passed to this code by the application. Let's forward the
            # error as an exception, so the App developor can act on it.
            raise ValueError(response.text)
    else:
        response.raise_for_status()


class Client:
    """A Client to lease semaphores from as Throttle service."""

    def __init__(self, base_url: str):
        self.base_url = base_url

    # Don't back off on timeouts. We might drain the semaphores by accident.
    @backoff.on_exception(backoff.expo, requests.ConnectionError)
    def acquire(self, semaphore: str, valid_for_sec: int = 300) -> Lease:
        body = {"semaphore": semaphore, "amount": 1, "valid_for_sec": valid_for_sec}
        response = requests.post(self.base_url + "/acquire", json=body, timeout=30)
        _raise_for_status(response)
        if response.status_code == 201:  # Created. We got a lease to the semaphore
            return Lease(id=response.text, pending=False)
        elif response.status_code == 202:  # Accepted. Ticket pending.
            id = response.text  # Ticket Id. Wait until it is promoted to lease
            return Lease(id=response.text, pending=True)

    @backoff.on_exception(backoff.expo, (requests.ConnectionError, requests.Timeout))
    def is_pending(self, lease: Lease, timeout_ms: int = 0):
        """
        Check if the lease is still pending. An optional timeout allows to block on a pending lease
        in order to wait for an active lease. Returns True if lease is still pending, False if the
        lease should be active.

        Raises UnknownLease in case the server doesn't know the lease (e.g. due to reboot of Throttle
        Server, or network timeouts of the heartbeats.)
        """

        response = requests.post(
            f"{self.base_url}/leases/{lease.id}/wait_on_pending?timeout_ms={timeout_ms}",
            timeout=timeout_ms / 1000 + 2,
        )

        _raise_for_status(response)

        lease.pending = json.loads(response.text)  # Returns boolean
        return lease.pending

    def remainder(self, semaphore: str) -> int:
        """
        The curent semaphore count. I.e. the number of available leases
        
        This is equal to the full semaphore count minus the current count. This number could become
        negative, if the semaphores have been overcommitted (due to previously reoccuring leases 
        previously considered dead).
        """
        response = requests.get(self.base_url + f"/remainder?semaphore={semaphore}")
        _raise_for_status(response)

        return int(response.text)

    def release(self, lease: Lease):
        """
        Deletes the lease on the throttle server.

        This is important to unblock other clients which may be waiting for the semaphore remainder
        to increase.
        """
        response = requests.delete(self.base_url + f"/leases/{lease.id}")
        _raise_for_status(response)

    def remove_expired(self):
        """Request the throttle server to remove all expired leases."""
        response = requests.post(self.base_url + "/remove_expired", timeout=30)
        _raise_for_status(response)


@contextmanager
def lease(client: Client, semaphore: str):
    lease = client.acquire(semaphore)
    while lease.pending:
        try:
            result = client.is_pending(lease, timeout_ms=5000)
        except UnknownLease:
            # The lease is unknown to the server. The server seems to have lost its state (e.g. due
            # to a reboot), since we created the lease with the last acquire. Let's handle this by
            # reacquiring the semaphore lease. It had been pending anyway.
            lease = client.acquire(semaphore)
    yield lease
    client.release(lease)
