# Throttle

Semaphores for distributed systems.

## Motivation

Throttle provides semaphores as a service via an http interface. This supports usecases like preventing access to shared resources from to many workers at once. Using Throttle is almost always some kind of workaround. In the best case the ressources (e.g. a Database) behaves well under load and is able to refuse connections. If the rest of the system is well able to handle the backpressure there would most likely be no need for Throttle. Yet, should you find yourself in need of a semaphore service Throttle might be for you.

## Local build

* Install Rust compiler and Cargo. Follow the instructions on
  [this site](https://www.rust-lang.org/en-US/install.html)
* Checkout the sources with

  ```bash
  ssh://git@code.blue-yonder.org:7999/labs/throttle.git
  ```

* You can run the build and run unit tests by executing

  ```bash
  cd throttle
  cargo test
  ```

* Execute integration test with clients

  ```bash
  cd python_client
  pip install -r requirements.txt
  pip install -r test-requirements.txt
  pytest
  ```
  