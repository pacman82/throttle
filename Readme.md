# Throttle

Semaphores for distributed systems.

## Motivation

Throttle provides semaphores as a service via an http interface. As the name indicates the primary usecase in mind is to throttle a systems access to a resource, by having the elements of that system to ask for permission (i.e. acquiring a lease) first. If the system consists of several process running on different machines, or virtual machines in the same Network, throttle might fit the bill.

Throttle aims to be easy to operate, well-behaved in edge cases and works without a persistence backend.

## Warning

Work in progress.

* Minimal Feature set (what's there works though)
* Not yet optimized
* Unstable interfaces

## Features

* Server builds and runs on Windows, Linux, and OS-X
* Python Client available
* Locks expire to prevent leaking semaphore count due to Network errors or client crashes
* Locks can be kept indefinetly by sending heartbeats of the server.
* No persistence backend required
  * Server recovers state from heartbeats in case of reboot

## Usage

### Operating a Throttle server

#### Starting and Shutdown

Assuming the throttle executable to be in your path environment variable, you start a throttle sever by executing it. You can display the availible command line options using the `--help` flag. Starting it without any arguments is going to boot the server with default configuration.

```bash
throttle
```

This starts the server in the current process. Navigate with a browser to `localhost:8000` to see a welcoming message. You can shut Throttle down gracefully by pressing `Ctrl + C`.

#### Default logging to stderr

Set the `THROTTLE_LOG` environment variable to see more output on standard error. Valid values are `ERROR`, `WARN`, `INFO`, `DEBUG` and `TRACE`.

In bash:

```bash
THROTTLE_LOG=WARN
```

or PowerShell:

```shell
$env:THROTLLE_LOG="WARN"
```

Starting the server now with an empty configuration yields a warning.

```log
[2020-03-03T19:02:42Z WARN  throttle] No semaphores configured.
```

*Hint:* Enabling Gelf logging currently disables logging to standard error.

#### Toml configuration file

To actually serve semaphores, we need to configure their names and full count. By default Throttle is looking for a configuration in the working directories `throttle.toml` file should it exist.

```toml
# Sample throttle.cfg Explaining the options

# The time interval in which the litter collection backgroud thread checks for expired leases.
# Default is set to 5 minutes.
litter_collection_interval = "5min"

[semaphores]
# Specify name and full count of semaphores. Below line creates a semaphore named A with a full
# count of 42. Setting the count to 1 would create a Mutex.
A = 42


# Optional logging config, to log into graylog
[logging.gelf]
name = "MyThrottleServer"
host = "my_graylog_instance.cloud"
port = 12201
## Set this to either ERROR, WARN, INFO, DEBUG or TRACE.
level = "INFO"
```

#### Metrics

Throttle supports Prometheus metrics, via the `/metrics` endpoint. Depending on your configuration and state they may e.g. look like this:

```prometheus
# HELP throttle_count Accumulated count of all active leases
# TYPE throttle_count gauge
throttle_count{semaphore="A"} 0
# HELP throttle_full_count New leases which would increase the count beyond this limit are pending.
# TYPE throttle_full_count gauge
throttle_full_count{semaphore="A"} 42
# HELP throttle_num_404 Number of Get requests to unknown resource.
# TYPE throttle_num_404 counter
throttle_num_404 0
# HELP throttle_pending Accumulated count of all pending leases
# TYPE throttle_pending gauge
throttle_pending{semaphore="A"} 0
```

### Python client

Throttle ships with a Python client. Here is how to use it in a nutshell.

```python
from throttle_client import Client, lock

# Configure endpoint to throttle server
c = Client("http://localhost:8000")

# Use client configuraton to acquire a lock (amount 1) to semaphore A
with lock(c, "A"):
    # Do stuff while holding lock to "A"
    # ...

# A is released at the end of with block
```

## Installation

### Server

The server binary is published to [crates.io](https://crates.io) and thus installable via cargo.

```bash
cargo install throttle-server
```

### Python Client

Python client is publish to [PyPi](https://pypi.org) and can be installed using pip.

```bash
pip install throttle-client
```

## Local build and test setup

* Install Rust compiler and Cargo. Follow the instructions on
  [this site](https://www.rust-lang.org/en-US/install.html)
* Checkout the sources with

  ```bash
  git clone https://github.com/pacman82/throttle.git
  ```

* You can run the build and run unit tests by executing

  ```bash
  cd throttle
  cargo test
  ```

* Execute integration test with clients

  ```bash
  cd python_client
  pip install -r test-requirements.txt
  pip install -e .
  pytest
  ```
