import json
from datetime import timedelta
from typing import Any, Dict

import requests
from tenacity import (Retrying, stop_after_attempt,  # type: ignore
                      wait_exponential)

from .status_code import is_recoverable_error as _is_recoverable_error


class Peer:
    """
    A peer of a throttle service. Lists active and pending leases for one thread of
    execution.
    """

    def __init__(
        self, id: str, active: Dict[str, int] = {}, pending: Dict[str, int] = {},
    ):
        self.id = id
        self.active = active
        self.pending = pending

    def has_pending(self) -> bool:
        """Returns true if this peer has pending leases."""
        return len(self.pending) != 0

    def has_active(self) -> bool:
        """Returns true if this peer has active leases."""
        return len(self.active) != 0

    def _make_active(self):
        """
        Call this to tell the lease that the pending admissions are now active
        """
        self.active.update(self.pending)
        self.pending = {}


def _translate_domain_errors(response: requests.Response):
    """
    Wrapper around Response.raise_for_status, translating domain specific errors
    """

    # Bad Request
    if response.status_code == 400:
        # Since this is the client code which made the erroneous request, it is
        # supposedly caused by wrong arguments passed to this code by the
        # application. Let's forward the error as an exception, so the App
        # developer can act on it.
        if "Unknown peer" == response.text:
            raise UnknownPeer()
        else:
            raise ValueError(response.text)
    # Conflict
    #
    # This is returned by the server e.g. if requesting a lock with a count higher than max.
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


class UnknownPeer(Exception):
    """
    The Throttle server does no longer know or has never known the peer id specified in
    this request. Possible causes are expiration of a peer on the server side, or a
    reboot of the server. To be robust against this kind of errors, the client can
    recover by using `new_peer` and restoring the state of the old one.
    """

    pass


class Client:
    """
    A Client to lease semaphores from a Throttle service.

    This class is conserned with the HTTP interface of the Server only. For a higher level interface
    look at the `lock` context manager.
    """

    def __init__(
        self, base_url: str, expiration_time: timedelta = timedelta(minutes=15),
    ):
        """
        * `base_url`: Url to the throttle server. E.g. https://my_throttle_instance:8000
        * `expiration_time`: If the heartbeat does not prolong the lifetime of this peer
        its locks are going to be released, after this timeout.
        """
        self.expiration_time = expiration_time
        self.base_url = base_url

    def _retrying(self) -> Any:
        """
        Configures tenacity for our needs. We don't store the object, since it is not pickable.
        """
        return Retrying(
            reraise=True,
            wait=wait_exponential(multiplier=1, min=4, max=32),
            stop=stop_after_attempt(10),
        )

    # TODO: How can we express a Function Signature, if using type annotations.
    def _try_request(self, send_request: Any) -> requests.Response:
        """
        Wraps `send_request` in some custom error handling for Requests.
        """
        for attempt in self._retrying():
            with attempt:
                response = send_request()
                # Witin `attempt` we raise only for recoverable errors. These must not be domain
                # errors, which implies that there is no need to translate them.
                if _is_recoverable_error(response.status_code):
                    response.raise_for_status()
        _translate_domain_errors(response)
        return response

    def new_peer(self, expires_in: timedelta = None) -> Peer:
        """
        Register a new peer with the server.

        A is used to acquire locks and keep the leases to them alive. A Peer owns the
        locks which it acquires and releasing it is going to release the owned locks as
        well.

        Every call to `new_peer` should be matched by a call to `release`.

        Creating a peer `new_peer` is separated from `acquire` in an extra Request
        mostly to make `acquire` idempotent. This prevents a call to acquire from
        acquiring more than one semaphore in case it is repeated due to a timeout.
        """
        if not expires_in:
            expires_in = self.expiration_time
        expires_in_str = _format_timedelta(expires_in)

        def send_new_peer():
            body = {
                "expires_in": expires_in_str,
            }
            return requests.post(self.base_url + "/new_peer", json=body, timeout=30)

        response = self._try_request(send_new_peer)
        return Peer(id=response.text, active={}, pending={})

    def acquire(
        self, peer: Peer, semaphore: str, count: int = 1, expires_in: timedelta = None
    ) -> bool:
        """
        Acquire a lock from the server.

        Every call to `acquire` should be matched by a call to `release`. Check out
        `lock` which as contextmanager does this for you.

        * `semaphore`: Name of the semaphore to be acquired.
        * `count`: The count of the lock. A larger count represents a larger 'piece' of
        the resource under procection.
        * `expires_in`: The amount of time the remains valid. Can be prolonged by
        calling heartbeat. After the time has passed the lock is considered released on
        the server side.

        Return `True` if the lock is active.
        """
        if not expires_in:
            expires_in = self.expiration_time
        expires_in_str = _format_timedelta(expires_in)

        def send_acquire():
            return requests.post(
                f"{self.base_url}/peer/{peer.id}/{semaphore}/acquire?expires_in={expires_in_str}",
                json=count,
                timeout=30,
            )

        response = self._try_request(send_acquire)
        if response.status_code == 201:  # Created. We got a lease to the semaphore
            peer.active = {semaphore: count}
            return True
        elif response.status_code == 202:  # Accepted. Ticket pending.
            peer.pending = {semaphore: count}
            return False
        else:
            # This should never be reached
            raise RuntimeError("Unexpected response from Server")

    def acquire_with_new_peer(
        self, semaphore: str, count: int = 1, expires_in: timedelta = None
    ) -> Peer:
        """
        Creates a new peer and acquires a lock from the server for it.

        Every call to `acquire` should be matched by a call to `release`. Check out `lock` which as
        contextmanager does this for you.

        * `semaphore`: Name of the semaphore to be acquired.
        * `count`: The count of the lock. A larger count represents a larger 'piece' of the
        resource under procection.
        * `expires_in`: The amount of time the remains valid. Can be prolonged by calling heartbeat.
        After the time has passed the lock is considered released on the server side.
        """
        peer = self.new_peer(expires_in=expires_in)
        self.acquire(peer, semaphore, count, expires_in)
        return peer

    def block_until_acquired(self, peer: Peer, block_for: timedelta) -> bool:
        """
        Block until all the leases of the peer are acquired, or the timespan specified
        in block_for has passed. Returns True if and only if the peer has leases which
        are still pending.
        """
        block_for_ms = int(block_for.total_seconds() * 1000)

        def send_request():
            response = requests.post(
                self.base_url
                + f"/peers/{peer.id}/block_until_acquired?timeout_ms={block_for_ms}",
                json={
                    "expires_in": "5min",
                    "active": peer.active,
                    "pending": peer.pending,
                },
                timeout=block_for_ms / 1000 + 30,
            )
            return response

        response = self._try_request(send_request)
        now_active = json.loads(response.text)
        if now_active:
            # Server told us all leases of this peer are active. Let's upate our local
            # bookeeping accordingly.
            peer._make_active()
        return peer.has_pending()

    def remainder(self, semaphore: str) -> int:
        """
        The curent semaphore count. I.e. the number of available leases

        This is equal to the full semaphore count minus the current count. This number
        could become negative, if the semaphores have been overcommitted (due to
        previously reoccuring leases previously considered dead).
        """

        def send_request():
            response = requests.get(self.base_url + f"/remainder?semaphore={semaphore}")
            return response

        response = self._try_request(send_request)
        return int(response.text)

    def is_acquired(self, peer: Peer) -> bool:
        """
        Ask the server wether all the locks associated with the peer are all acquired.
        """

        def send_request():
            response = requests.get(self.base_url + f"/peers/{peer.id}/is_acquired")
            return response

        response = self._try_request(send_request)
        return json.loads(response.text)

    def freeze(self, time: timedelta):
        """
        Freezes the server state for specified amount of time.

        Yes, it propably is a bad idea to call this in production code. Yet it is useful for
        testing.
        """

        def send_request():
            response = requests.post(
                self.base_url + f"/freeze?for={_format_timedelta(time)}"
            )
            return response

        self._try_request(send_request)

    def release(self, peer: Peer):
        """
        Deletes the peer on the throttle server.

        This is important to unblock other clients which may be waiting for the
        semaphore remainder to increase.
        """

        def send_request():
            response = requests.delete(self.base_url + f"/peers/{peer.id}")
            return response

        self._try_request(send_request)

    def remove_expired(self) -> int:
        """
        Request the throttle server to remove all expired peers.

        Returns number of expired peers.
        """

        def send_request():
            response = requests.post(self.base_url + "/remove_expired", timeout=30)
            return response

        response = self._try_request(send_request)
        # Number of expired peers
        return json.loads(response.text)

    def heartbeat(self, peer: Peer):
        """
        Sends a PUT request to the server, updating the expiration timestamp and
        repeating the information of the peer.
        """
        assert (
            not peer.has_pending()
        ), "Peers with pending leases must not be kept alive via heartbeat."

        def send_request():
            response = requests.put(
                f"{self.base_url}/peers/{peer.id}",
                json={
                    "expires_in": _format_timedelta(self.expiration_time),
                    "active": peer.active,
                },
                timeout=30,
            )
            return response

        self._try_request(send_request)
