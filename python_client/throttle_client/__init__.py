from contextlib import contextmanager
from datetime import timedelta
from threading import Event, Thread
from time import time
from typing import Iterator, Optional

import requests

from .client import Client, Peer, UnknownPeer


# Heartbeat is implemented via an event, rather than a thread with a sleep, so we can
# interupt and it, then the Application code wants to release the semaphores without
# waiting for the current interval to finish
class Heartbeat:
    def __init__(self, client: Client, peer: Peer, interval: timedelta):
        self.client = client
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
                self.client.heartbeat(self.peer)
            except UnknownPeer:
                self.client.restore(self.peer)
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
    * `heartbeat_interval`: Default interval for reneval of peer. Setting it to `None` will
    deactivate the heartbeat.
    """
    # Recover from `UnknownPeer` if server forgets state between `new_peer` and `acquire`
    while True:
        peer = client.new_peer()
        try:
            _ = client.acquire(peer, semaphore, count=count)
        except UnknownPeer:
            # Peer has no prior state, just repeart acquiring the lock
            continue
        break
    # Remember this moment in order to figure out later how much time has passed since
    # we started to acquire the lock
    start = time()
    # We pass this as a parameter to the throttle server. It will wait for this amount of time
    # before answering, that the lease is still pending. In case the lease can be acquired it is
    # still going to answer immediatly, of course.
    block_for = timedelta(seconds=5)
    while peer.has_pending():
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
            _ = client.acquire(
                peer, semaphore, count=count, block_for=block_for
            )
        except UnknownPeer:
            client.restore(peer)

    # Yield and have the heartbeat in an extra thread, during it being active.
    if heartbeat_interval is not None:
        heartbeat = Heartbeat(client, peer, heartbeat_interval)
        heartbeat.start()
    yield peer
    if heartbeat_interval is not None:
        heartbeat.stop()

    try:
        client.release(peer)
    except requests.ConnectionError:
        # Ignore recoverable errors. `release` retried alread. The litter collection on
        # server side, takes care of freeing the lease.
        pass
