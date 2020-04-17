from contextlib import contextmanager
from datetime import timedelta
from threading import Event, Thread
from time import time
from typing import Dict, Iterator, Optional

import requests

from .client import Client, UnknownPeer


class Peer:
    """
    A peer of a throttle service.

    For the most part this class just holds the peer's id, used to identify it on the
    server side. If all goes well, this is all that is needed. Yet to facilitate
    error handling in case of a server reboot / crash, we also remember the acquired
    locks on the client side.
    """

    def __init__(
        self,
        client: Client,
        id: Optional[int] = None,
        acquired: Optional[Dict[str, int]] = None,
        expiration_time: Optional[timedelta] = None,
    ):
        if expiration_time:
            self.expiration_time = expiration_time
        else:
            self.expiration_time = timedelta(minutes=5)

        self.client = client

        if id is None:
            self.id = client.new_peer(expires_in=self.expiration_time)
        else:
            self.id = id

        # The server remembers acquired locks, but we also keep it on the client side, so
        # we can recover it in case the server looses the state.
        if acquired is None:
            self.acquired = {}
        else:
            self.acquired = acquired

    @classmethod
    def from_server_url(cls, baseurl: str):
        return cls(client=Client(base_url=baseurl))

    def acquire(self, semaphore, count=1, block_for: timedelta = None) -> bool:
        """
        Acquire a lock from the server.

        Every call to `acquire` should be matched by a call to `release`. Check out
        `lock` which as contextmanager does this for you.

        * `semaphore`: Name of the semaphore to be acquired.
        * `count`: The count of the lock. A larger count represents a larger 'piece' of
        the resource under procection.
        * `block_for`: The request returns as soon as the lock could be acquireod or
        after the duration has elapsed, even if the lock could not be acquired. If set to
        `None`, the request returns immediatly.

        Return `True` if the lock is active.
        """
        if self.client.acquire(
            self.id,
            semaphore,
            count=count,
            block_for=block_for,
            expires_in=self.expiration_time,
        ):
            # Remember that we acquired that lock, so heartbeat can restore it, if need
            # be.
            self.acquired[semaphore] = count
            return True
        else:
            return False

    def restore(self):
        """
        Restores the information of this peer to the server in case the server lost its state, or
        the communication had been interrupted and the peer expired. Usually called as a reaction
        to a 'Unkown Peer' error.
        """
        self.client.restore(self.id, self.acquired, expires_in=self.expiration_time)

    def release(self, semaphore: str):
        """
        Release lock to semaphore.
        """
        self.client.release_lock(self.id, semaphore)

    def heartbeat(self):
        """Send heartbeat to server, so the peer does not expire"""
        self.client.heartbeat(self.id, expires_in=self.expiration_time)

    def has_acquired(self) -> bool:
        """`True` if this peer has acquired locks."""
        return len(self.acquired) != 0

    def remove_from_server(self):
        """Delete the peer from the server"""
        self.client.release(self.id)


# Heartbeat is implemented via an event, rather than a thread with a sleep, so we can
# interupt and it, then the Application code wants to release the semaphores without
# waiting for the current interval to finish
class Heartbeat:
    def __init__(self, peer: Peer, interval: timedelta):
        self.peer = peer
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
        self.cancel.wait(self.interval_sec)
        while self.peer.has_acquired() and not self.cancel.is_set():
            try:
                self.peer.heartbeat()
            except UnknownPeer:
                self.peer.restore()
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
    peer: Peer,
    semaphore: str,
    count: int = 1,
    heartbeat_interval: Optional[timedelta] = timedelta(minutes=5),
    timeout: Optional[timedelta] = None,
) -> Iterator[Peer]:
    """
    Acquires a lock to a semaphore

    ## Keyword arguments:

    * `count`:  Lock count. May not exceed the full count of the semaphore
    * `timeout`: Leaving this at None, let's the lock block until the lock can be acquired. Should a
    timeout be specified the call is going to raise a `Timeout` exception should it exceed before
    the lock is acquired.
    * `peer`: Peer to use than acquiring a lock. The default `None` is to create a new one.
    * `heartbeat_interval`: Default interval for reneval of peer. Setting it to `None` will
    deactivate the heartbeat.
    """
    # Remember this moment in order to figure out later how much time has passed since
    # we started to acquire the lock
    start = time()
    # We pass this as a parameter to the throttle server. It will wait for this amount of time
    # before answering, that the lease is still pending. In case the lease can be acquired it is
    # still going to answer immediatly, of course.
    block_for = timedelta(seconds=5)

    while True:
        if timeout:
            # The time between now and start is the amount of time we are waiting for the
            # lock.
            now = time()
            passed = timedelta(seconds=now - start)
            # Figure out if the lock timed out
            if timeout < passed:
                raise Timeout
            # If we time out in a timespan < 5 seconds, we want to block only for the time
            # until the timeout.
            else:
                block_for = min(timeout - passed, block_for)

        try:
            if peer.acquire(semaphore, count=count, block_for=block_for):
                # Remember that we acquired that lock, so heartbeat can restore it, if need be.
                peer.acquired[semaphore] = count
                break

        except UnknownPeer:
            peer.restore()

    # Yield and have the heartbeat in an extra thread, during it being active.
    if heartbeat_interval is not None:
        heartbeat = Heartbeat(peer, heartbeat_interval)
        heartbeat.start()
    try:
        yield peer
    finally:
        if heartbeat_interval is not None:
            heartbeat.stop()

        try:
            assert peer.acquired.pop(semaphore) == count
            if peer.acquired:
                # Acquired dict still holds locks, remove only this one
                peer.release(semaphore)
            else:
                # No more locks associated with this peer. Let's remove it entirely
                peer.remove_from_server()

        except requests.ConnectionError:
            # Ignore recoverable errors. `release` retried alread. The litter collection on
            # server side, takes care of freeing the lease.
            pass
