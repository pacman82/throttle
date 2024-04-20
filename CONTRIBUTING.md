# Contributions

Wether they be in code, intersting feature suggestions, design critique or bug reports, all contributions are welocme. Please start an issue, before investing a lot of work. This helps avoid situations there I would feel the need to reject a large body of work, and a lot of your time has been wasted. Throttle is a pet project and a work of love, which implies that I maintain it in my spare time. Please understand that I may not always react immediatly. If you contribute code to fix a Bug, please also contribute the test to fix it. Happy contributing.

## Local build and test setup

* Install Rust compiler and Cargo. Follow the instructions on [this site](https://www.rust-lang.org/en-US/install.html).
* Checkout the sources.

  ```bash
  git clone https://github.com/pacman82/throttle.git
  ```

* Build the throttle server, run the unit tests and the integration tests with the Rust client.

  ```bash
  cd throttle
  cargo test
  ```

* Execute integration of throttle server with Python client.

  ```bash
  cd python_client
  pip install -e .[test]
  pytest
  ```
