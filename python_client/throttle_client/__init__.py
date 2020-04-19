from contextlib import contextmanager
from datetime import timedelta
from threading import local
from time import time
from typing import Iterator, Optional

import requests

from .client import UnknownPeer, Client
from .peer import Peer, PeerWithHeartbeat


# Silence flake8 warning about unused import
_reexport = [Client]

# Keep track of peers local thread. This should usually be only contain one instance
# each, because there should be only one throttle server. Having the peer thread local is
# a sensible default, since lock hierachies are enforced on peer level. So you want to
# have one peer for each thread.
threadlocal = local()
threadlocal.peers = {}


def local_peer(url: str) -> Peer:
    """
    Gets an existing, or creates a new thread local peer instance.

    ## Keyword arguments:

    * `url`: Base url to the throttle server. E.g. `http://localhost:8000/`
    """
    if url not in threadlocal.peers:
        peer = PeerWithHeartbeat.from_server_url(url)
        threadlocal.peers[url] = peer
    return threadlocal.peers[url]


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
    if hasattr(peer, "start_heartbeat"):
        peer.start_heartbeat()
    try:
        yield peer
    finally:
        if hasattr(peer, "stop_heartbeat"):
            peer.stop_heartbeat()

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
