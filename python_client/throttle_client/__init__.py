import requests
import backoff
import json
from typing import Dict
from contextlib import contextmanager
from threading import Event, Thread


class Lease:
    """
    A lease from a throttle service. Lists active and pending resources for
    one thread of execution.
    """

    def __init__(
        self,
        id: str,
        active: Dict[str, int] = {},
        pending: Dict[str, int] = {},
    ):
        self.id = id
        self.active = active
        self.pending = pending

    def has_pending(self) -> bool:
        """Returns true if this lease has pending admissions."""
        return len(self.pending) != 0

    def has_active(self) -> bool:
        """Returns true if this lease has active admissions."""
        return len(self.active) != 0

    def _make_active(self):
        """
        Call this to tell the lease that the pending admissions are now active
        """
        self.active.update(self.pending)
        self.pending = {}


def _raise_for_status(response: requests.Response):
    """
    Wrapper around Response.raise_for_status, translating 4xx to value errors
    """

    if response.status_code == 400:
        # Since this is the client code which made the erroneous request, it is
        # supposedly caused by wrong arguments passed to this code by the
        # application. Let's forward the error as an exception, so the App
        # developer can act on it.
        raise ValueError(response.text)
    else:
        response.raise_for_status()


class Client:
    """A Client to lease semaphores from as Throttle service."""

    def __init__(
        self, base_url: str, expiration_time_sec: int = 900,
    ):
        self.expiration_time_sec = expiration_time_sec
        self.base_url = base_url

    # Don't back off on timeouts. We might drain the semaphores by accident.
    @backoff.on_exception(backoff.expo, requests.ConnectionError)
    def acquire(self, semaphore: str, valid_for_sec: int = None) -> Lease:
        if not valid_for_sec:
            valid_for_sec = self.expiration_time_sec
        amount = 1
        body = {
            "pending": {semaphore: amount},
            "valid_for_sec": valid_for_sec,
        }
        response = requests.post(
            self.base_url + "/acquire", json=body, timeout=30
        )
        _raise_for_status(response)
        if (
            response.status_code == 201
        ):  # Created. We got a lease to the semaphore
            return Lease(id=response.text, active={semaphore: amount})
        elif response.status_code == 202:  # Accepted. Ticket pending.
            return Lease(id=response.text, pending={semaphore: amount})

    @backoff.on_exception(
        backoff.expo, (requests.ConnectionError, requests.Timeout)
    )
    def wait_for_admission(self, lease: Lease, timeout_ms: int = 0):
        """
        Check if the lease is still pending. An optional timeout allows to
        block on a pending lease in order to wait for an active lease. Returns
        True if lease is still pending, False if the lease should be active.

        Raises UnknownLease in case the server doesn't know the lease (e.g. due
        to reboot of Throttle Server, or network timeouts of the heartbeats.)
        """

        response = requests.post(
            self.base_url
            + f"/leases/{lease.id}/wait_on_admission?timeout_ms={timeout_ms}",
            json={
                "valid_for_sec": 300,
                "active": lease.active,
                "pending": lease.pending,
            },
            timeout=timeout_ms / 1000 + 2,
        )

        _raise_for_status(response)

        now_active = json.loads(response.text)
        if now_active:
            # Server told us all admissions in the lease are active. Let's
            # update our local bookeeping accordingly.
            lease._make_active()
        return lease.has_pending()

    def remainder(self, semaphore: str) -> int:
        """
        The curent semaphore count. I.e. the number of available leases

        This is equal to the full semaphore count minus the current count. This
        number could become negative, if the semaphores have been overcommitted
        (due to previously reoccuring leases previously considered dead).
        """
        response = requests.get(
            self.base_url + f"/remainder?semaphore={semaphore}"
        )
        _raise_for_status(response)

        return int(response.text)

    def release(self, lease: Lease):
        """
        Deletes the lease on the throttle server.

        This is important to unblock other clients which may be waiting for
        the semaphore remainder to increase.
        """
        try:
            response = requests.delete(self.base_url + f"/leases/{lease.id}")
            _raise_for_status(response)
        except requests.ConnectionError:
            # Let's not wait for the server. This lease is a case for the
            # litter collection.
            pass

    def remove_expired(self) -> int:
        """
        Request the throttle server to remove all expired leases.

        Returns number of expired leases.
        """
        response = requests.post(self.base_url + "/remove_expired", timeout=30)
        _raise_for_status(response)
        # Number of expired leases
        return json.loads(response.text)

    def heartbeat(self, lease: Lease):
        """
        Sends a PUT request to the server, updating the expiration timestamp
        and repeating the information in the lease.
        """
        assert (
            not lease.has_pending()
        ), "Pending leases must not be kept alive via heartbeat."
        response = requests.put(
            f"{self.base_url}/leases/{lease.id}",
            json={
                "valid_for_sec": self.expiration_time_sec,
                "active": lease.active,
            },
            timeout=30,
        )
        _raise_for_status(response)


# Heartbeat is implemented via an event, rather than a thread with a sleep, so
# we can interupt and it, then the Application code wants to release the
# semaphores without waiting for the current interval to finish
class Heartbeat:
    def __init__(self, client: Client, lease: Lease, interval_sec: float):
        self.client = client
        self.lease = lease
        self.interval_sec = interval_sec
        self.cancel = Event()
        self.thread = Thread(target=self._run)

    def start(self):
        self.cancel.clear()
        self.thread.start()

    def stop(self):
        self.cancel.set()
        self.thread.join()

    def _run(self):
        while self.lease.has_active() and not self.cancel.is_set():
            try:
                self.client.heartbeat(self.lease)
            except requests.ConnectionError:
                pass
            self.cancel.wait(self.interval_sec)


@contextmanager
def lease(client: Client, semaphore: str, heartbeat_interval_sec: float = 300):
    lease = client.acquire(semaphore)
    while lease.has_pending():
        _ = client.wait_for_admission(lease, timeout_ms=5000)
    heartbeat = Heartbeat(client, lease, heartbeat_interval_sec)
    heartbeat.start()
    yield lease
    heartbeat.stop()
    client.release(lease)
