# Throttle

Semaphores for distributed systems.

## Motivation

Throttle provides semaphores as a service via an http interface. As the name indicates the primary usecase in mind is to throttle a systems access to a resource, by having the elements of that system to ask for permission (i.e. acquiring a lease) first. If the system consists of several process running on different machines, or virtual machines in the same Network, throttle might fit the bill.

Throttle aims to be easy to operate, well-behaved in edge cases and works without a persistence backend.

## Features

* Server builds and runs on Windows, Linux, and OS-X.
* Clients
  * Python: A high level client with blocking API.
  * Rust: Low level client with async API.
* Prevents Deadlocks, through enforcing lock hierarchies.
* Fairness (longer waiting peers have priority)
* Locks with large count, won't be starved by lots of others with a small counts.
* Resilient against Network outages.
  * Locks expire to prevent leaking semaphore count due to Network errors or client crashes.
  * Locks can be prolonged indefinetly using heartbeats which are send to the server.
* Observability
  * Prometheus Metrics
  * Logging to stderr
* No persistence backend is required.
  * Server keeps state in memory.
  * Clients restore state to the server, in case of server reboot.

## Installation

### Server

#### Run with Docker

The throttle sever is also released as a small container image to docker hub.

```shell
docker pull pacman82/throttle
```

Assuming you have a `throttle.toml` configuration file in the current working directory you could then run the server using:

```shell
docker run --rm -v ${PWD}:/cfg -p 8000:8000 pacman82/throttle -c cfg/throttle.toml
```

#### Download binary release

For Windows, OS-X and Linux there are prebuild binaries available together with the GitHub release. You can find the latest release at <https://github.com/pacman82/throttle/releases/latest>.

#### Install with Cargo

The server binary is published to [crates.io](https://crates.io) and thus installable via cargo.

```bash
cargo install throttle-server
```

### Python Client

Python client is published to [PyPi](https://pypi.org) and can be installed using pip.

```bash
pip install throttle-client
```

## Usage

### Operating a Throttle server

#### Starting and Shutdown

Assuming the throttle executable to be in your path environment variable, you start a throttle sever by executing it. You can display the availible command line options using the `--help` flag. Starting it without any arguments is going to boot the server with default configuration.

```bash
throttle
```

This starts the server in the current process. Navigate with a browser to `localhost:8000` to see a welcoming message. You can shut Throttle down gracefully by pressing `Ctrl + C`.

#### Logging to stderr

Set the `THROTTLE_LOG` environment variable to see more output on standard error. Valid values are `ERROR`, `WARN`, `INFO`, `DEBUG` and `TRACE`.

In bash:

```bash
THROTTLE_LOG=WARN
```

or PowerShell:

```shell
$env:THROTTLE_LOG="INFO"
```

Starting the server now yields more information.

```log
[2020-04-12T18:56:23Z INFO  throttle] Hello From Throttle
[2020-04-12T18:56:23Z WARN  throttle] No semaphores configured.
```

#### Toml configuration file

To actually serve semaphores, we need to configure their names and full count. By default Throttle is looking for a configuration in the working directories `throttle.toml` file should it exist.

```toml
# Sample throttle.cfg Explaining the options

[semaphores]
# Specify name and full count of semaphores. Below line creates a semaphore named A with a full
# count of 42. Setting the count to 1 would create a Mutex.
A = 42

## Optional logging config, to log to stderr. Can be overwritten using the `THROTTLE_LOG`
## environment variable.
# [logging.stderr]
# Set this to either ERROR, WARN, INFO, DEBUG or TRACE.
# level = "INFO"
```

#### Metrics

Throttle supports Prometheus metrics, via the `/metrics` endpoint. Depending on your configuration and state they may e.g. look like this:

```prometheus
# HELP throttle_acquired Sum of all acquired locks.
# TYPE throttle_acquired gauge
throttle_acquired{semaphore="A"} 0
# HELP throttle_longest_pending_sec Time the longest pending peer is waiting until now, to acquire a lock to a semaphore.
# TYPE throttle_longest_pending_sec gauge
throttle_longest_pending_sec{semaphore="A"} 0
# HELP throttle_max Maximum allowed lock count for this semaphore.
# TYPE throttle_max gauge
throttle_max{semaphore="A"} 42
# HELP throttle_num_404 Number of Get requests to unknown resource.
# TYPE throttle_num_404 counter
throttle_num_404 0
# HELP throttle_pending Sum of all pending locks
# TYPE throttle_pending gauge
throttle_pending{semaphore="A"} 0
```

### Python client

Throttle ships with a Python client. Here is how to use it in a nutshell.

```python
from throttle_client import Peer, lock

# Configure endpoint to throttle server
url = "http://localhost:8000"

# Acquire a lock (with count 1) to semaphore A
with lock(url, "A"):
    # Do stuff while holding lock to "A"
    # ...

# For acquiring lock count != 1 the count can be explicitly specified.
with lock(url, "A", count=4):
    # Do stuff while holding lock to "A"
    # ...

# A is released at the end of with block
```

### Preventing Deadlocks with lock hierarchies

Assume two semaphores `A` and `B`.

```toml
[semaphores]
A = 1
B = 1
```

You want to acquire locks to them in a nested fashion:

```python
from throttle_client import Peer, lock

# Configure endpoint to throttle server
url = "http://localhost:8000"

# Acquire a lock to semaphore A
with lock(url, "A"):
    # Do stuff while holding lock to "A"
    # ...
    with lock(url, "B") # <-- This throws an exception: "Lock Hierarchy Violation".
      # ...

```

The throttle server helps you preventing deadlocks. If `A` and `B` are not always locked in the same
order, your system might deadlock at some point. Such errors can be hard to Debug, which is why
throttle fails early at any chance of deadlock. To enable the use case above, give `A` a lock level
higher than `B`.

```toml
[semaphores]
A = { max=1, level=1 }
# Level 0 is default. So the short notation is still good for B.
B = 1
```

### Http routes

* GET `/`: Prints a greeting message
* GET `/health`: Always answers with `200 OK`
* GET `/metrics:`: Metrics for prometheus
* GET `/version`: Returns server version.

#### Routes for managing peers and locks

* POST `new_peer`: Creates a new peer. The body to this request must contain a human readable time duration with dimension in quotes. E.g.: `"expires_in": "5m"`, `"expires_in": "30s"` or `"expires_in": "12h"`. This is the time after which the peer is going to expire if not kept alive by prolonging its expiration time. Every lock acquired is always associated with a peer. If a peer expires, all locks are released. The request returns a random integer as peer id.
* DELETE `/peers/{id}`: Removes the peer, releasing all its locks in the process. Every call to `new_peer` should be matched by a call to this route, so other peers do not have to wait for this peer to expire in order to acquire locks to the same semaphores.
* PUT `/peers/{id}/{semaphore}`: Acquires lock to a semaphore for an existing peer. The body must contain the desired lock count. Throttle will answer either with `200 Ok` in case the lock could be acquired, or `202 Accepted` in case the lock can not be acquired until other peers release their lock. Specifying a lock count higher than the full count of the lock message or violating lock hierarchy will result in a `409 Conflict` error. Requesting a lock for an unknown semaphore or unknown peer is going to result in `400 Bad Request`. This request is idempotent, so acquiring locks can be repeated in case of a timeout, without risk of draining the semaphore. If waiting for a lock on the client side, busy waiting can be avoided using the optional `block_for` query parameter. E.g. `/peer/{id}/{semaphore}?block_for=10s`. The semantics for acquiring a lock with count `0` would be akward, so it's forbidden for now.
* DELETE `/peers/{id}/{semaphore}`: Releases one specific lock for a peer.
* POST `/restore`: Can be used by the client to react to a `400 Bad Request` those body contains `Unknown Semaphore`. This error indicates that the server does not remeber the clients state (e.g. the client may have expired due to prolonged connection loss). In this situation the client may choose to restore its previous state and acquired locks to the server. The body contains a JSON like this:

  ```json
  {
    "expires_in": "5m",
    "peer_id": 42,
    "acquired": {
      "A": 3,
      "B": 1
    }
  }
  ```

  This would restore a client with id `42` and a lifetime of 5 minutes. It has a lock with count 3 to `A` and one with count 1 to `B`.
* `Get` `/remainder?semaphore={semaphore}`: Answers the maximum semaphore count minus the sum of all acquired locks for this semaphore. Response is a plain text integer.
* `Get` `/peers/{id}/is_acquired`: Answers `false` if peer has a pending lock. If all the locks of the peer are acquired the answer is `true`.
