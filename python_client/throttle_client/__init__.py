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
        self, id: int, client: Client, acquired: Dict[str, int],
    ):
        self.id = id
        self.client = client
        # The server does also keep this state, but we also keep it on the client side,
        # so we can recover it in case the server looses the state.
        self.acquired = acquired

    def has_acquired(self) -> bool:
        """`True` if this peer has acquired locks."""
        return len(self.acquired) != 0

    def _acquired(self, semaphore: str, count: int):
        """
        Remember that the pending lock is now acquired.
        """
        self.acquired.update({semaphore: count})

    def heartbeat(self):
        self.client.heartbeat(self.id)
    
    def restore(self):
        self.client.restore(self.id, self.acquired)


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


@contextmanager
def heartbeat(peer: Peer, interval: timedelta) -> Iterator[Heartbeat]:
    """
        Heartbeat context manager, manages starting/stopping of heartbeat.
    """
    _heartbeat = Heartbeat(peer, interval)
    _heartbeat.start()
    try:
        yield _heartbeat
    finally:
        _heartbeat.stop()


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
    peer: Optional[Peer] = None,
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
    if peer is None:
        peer_id = client.new_peer()
        peer = Peer(peer_id, client, acquired={})
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
            if client.acquire(peer.id, semaphore, count=count, block_for=block_for):
                # Remember that we acquired that lock, so heartbeat can restore it, if need be.
                peer.acquired[semaphore] = count
                break

        except UnknownPeer:
            peer.restore()

    
    try:
        if heartbeat_interval is not None:
            with heartbeat(peer, heartbeat_interval):
                # Yield and have the heartbeat in an extra thread, during it being active.
                yield peer
        else:
            # Yield without heartbeat
            yield peer
    finally:        
        try:
            assert peer.acquired.pop(semaphore) == count
            if peer.acquired:
                # Acquired dict still holds locks, remove only this one
                client.release_lock(peer.id, semaphore)
            else:
                # No more locks associated with this peer. Let's remove it entirely
                client.release(peer.id)

        except requests.ConnectionError:
            # Ignore recoverable errors. `release` retried alread. The litter collection on
            # server side, takes care of freeing the lease.
            pass
