import requests
import backoff
import json
from typing import Dict, Optional
from contextlib import contextmanager
from threading import Event, Thread
from datetime import timedelta
from time import time, sleep


class Lease:
    """
    A lease from a throttle service. Lists active and pending resources for
    one thread of execution.
    """

    def __init__(
        self, id: str, active: Dict[str, int] = {}, pending: Dict[str, int] = {},
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
    Wrapper around Response.raise_for_status, translating domain specific errors
    """

    if response.status_code == 400:
        # Since this is the client code which made the erroneous request, it is
        # supposedly caused by wrong arguments passed to this code by the
        # application. Let's forward the error as an exception, so the App
        # developer can act on it.
        raise ValueError(response.text)
    # Conflict
    elif response.status_code == 409:
        raise ValueError(response.text)
    else:
        response.raise_for_status()


def _format_timedelta(interval: timedelta) -> str:
    """
    Convert a python timedelta object into a string representation suitable for
    the throttle server.
    """
    total_milliseconds = int(interval.total_seconds() * 1000)
    return f"{total_milliseconds}ms"


class Client:
    """A Client to lease semaphores from as Throttle service."""

    def __init__(
        self, base_url: str, expiration_time: timedelta = None,
    ):
        if expiration_time:
            self.expiration_time = expiration_time
        else:
            self.expiration_time = timedelta(minutes=15)
        self.base_url = base_url

    # Don't back off on timeouts. We might drain the semaphores by accident.
    @backoff.on_exception(backoff.expo, requests.ConnectionError)
    def acquire(
        self, semaphore: str, count: int = 1, expires_in: timedelta = None
    ) -> Lease:
        if not expires_in:
            expires_in = self.expiration_time
        body = {
            "pending": {semaphore: count},
            "expires_in": _format_timedelta(expires_in),
        }
        response = requests.post(self.base_url + "/acquire", json=body, timeout=30)
        _raise_for_status(response)
        if response.status_code == 201:  # Created. We got a lease to the semaphore
            return Lease(id=response.text, active={semaphore: count})
        elif response.status_code == 202:  # Accepted. Ticket pending.
            return Lease(id=response.text, pending={semaphore: count})

    def wait_for_admission(self, lease: Lease, block_for: timedelta) -> bool:
        """
        Check if the lease is still pending. Returns True if lease is still pending,
        False if the lease should be active.

        Raises UnknownLease in case the server doesn't know the lease (e.g. due
        to reboot of Throttle Server, or network timeouts of the heartbeats.)
        """
        block_for_ms = int(block_for.total_seconds() * 1000)

        response = requests.post(
            self.base_url
            + f"/leases/{lease.id}/block_until_acquired?timeout_ms={block_for_ms}",
            json={
                "expires_in": "5min",
                "active": lease.active,
                "pending": lease.pending,
            },
            timeout=block_for_ms / 1000 + 2,
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
        response = requests.get(self.base_url + f"/remainder?semaphore={semaphore}")
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
                "expires_in": _format_timedelta(self.expiration_time),
                "active": lease.active,
            },
            timeout=30,
        )
        _raise_for_status(response)


# Heartbeat is implemented via an event, rather than a thread with a sleep, so we can
# interupt and it, then the Application code wants to release the semaphores without
# waiting for the current interval to finish
class Heartbeat:
    def __init__(self, client: Client, lease: Lease, interval: timedelta):
        self.client = client
        self.lease = lease
        # Interval in between heartbeats for an active lease
        self.interval_sec = interval.total_seconds()
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


class Timeout(Exception):
    """
    Thrown by lock in order to indicate that the specified amount time has passed and
    the call has given up on acquiring the lock
    """

    pass


@contextmanager
def lock(
    client: Client,
    semaphore: str,
    count: int = 1,
    heartbeat_interval: timedelta = timedelta(minutes=5),
    timeout: Optional[timedelta] = None,
) -> Lease:
    """
    Acquires a lock to a semaphore

    Keyword arguments:
    count -- Lock count. The lock counts of all leases combined may not exceed the
             semaphore count.
    timeout -- Leaving this at None, let's the lock block until the lock can be
               acquired. Should a timeou be specified the call is going to raise a
               Timeout exception should it exceed before the lock is acquired.
    """
    lease = client.acquire(semaphore, count=count)
    # Remember this moment in order to figure out later how much time has passed since
    # we started to acquire the lock
    start = time()
    while lease.has_pending():
        # The time between now and start is the amount of time we are waiting for the
        # lock.
        now = time()
        passed = timedelta(seconds=now - start)
        # Figure out if the lock timed out
        if timeout < passed:
            raise Timeout
        # If we time out in a timespan < 5 seconds, we want to block only for the time
        # until the timeout.
        elif timeout - passed < timedelta(seconds=5):
            block_for = timeout - passed
        else:
        # Even if the timeout is langer than 5 seconds, block only for that long, since
        # we do not want to keep the http request open for to long.
            block_for = timedelta(seconds=5)
        try:
            _ = client.wait_for_admission(lease, block_for)
        # Catch ConnectionError and Timeout, to be able to recover pending leases
        # in case of a server reboot.
        except requests.ConnectionError:
            sleep(block_for.total_seconds())
        except requests.Timeout:
            pass

    heartbeat = Heartbeat(client, lease, heartbeat_interval)
    heartbeat.start()
    yield lease
    heartbeat.stop()
    client.release(lease)
