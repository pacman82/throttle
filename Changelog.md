Changelog
=========

0.1.0
-----

Initial Release

0.1.1
-----

Fixes a type error in the python client occuring then checking the timeout for a pending lock if the timeout is set to `None`.

0.1.2
-----

Fix: pip installing python client now also installs `requests` dependencies, which has been missing from `install_requires`.

0.1.3
-----

* Python Client: `lock` will no longer throw in case of a timeout during the release of the semaphore.
