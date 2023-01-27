Changelog
=========

0.4.1
-----

* Updated dependencies

0.4.0
-----

* Updated dependencies
* Dropped support for logging to graylog.

0.3.17
------

* Updated dependencies

0.3.16
------

* Fix: Fixed an issue with Semaphore names containing slashes or ampersands would not always be correctly percent decoded if passed in URL paths.
* Updated dependencies

0.3.15
------

* Updated dependencies
* Deployed `throttle` image to dockerhub

0.3.14
------

* Updated dependencies

0.3.13
------

* Updated dependencies

0.3.9-12
--------

* Test release process, no changes.

0.3.8
-----

* Add Rust client http layer

0.3.7
-----

* Statically link C runtime for windows.
* Updated dependencies

0.3.6
-----

* Publish server wheels to pypi

0.3.1-5
-------

* Test release - no changes

0.3.0
-----

* Lock hierachies are enforced.

0.2.0
-----

* High level python interface entry points are now `Peer` and `Lock` rather than `Client` and `Lock`.
* Removed HTTP route `/freeze`.
* Recover from unknown peer is now handled on the client side.
  * Uses new route `/restore`.
* Removed `Peer.has_pending()`.
* Acquiring locks is now idempotent
* One peer can now hold multiple locks
* Log level to stderr can now be configured in `throttle.toml`.

0.1.9
-----

* Fix: Pending leases are now acquired immediatly. Previously their acquiration could have been
delayed if the peer holding the lock previously did expire, rather than release its lock.
* New route `is_acquired` tells if all the locks of a peer could be acquired.

0.1.8
-----

* Fix: Releasing locks, did fail to stop other locks from pending if a higher priority lock had
already been acquired.
* Favicon route is now `favicon.ico` instead of `favicon`.
* Add route `version` to display current version number.

0.1.7
-----

* Fix: 0.1.6 introduced a behaviour, there acquiring a lock would always fail initialy, for all but
  the first peer.

0.1.6
-----

* Fairness
* Large semaphores don't starve

0.1.5
-----

* Python Client: `Client` can now be pickeled again.

0.1.4
-----

* Python Client: Use tenacity for all requests

0.1.3
-----

* Python Client: `lock` will no longer throw in case of a timeout during the release of the semaphore.

0.1.2
-----

Fix: pip installing python client now also installs `requests` dependencies, which has been missing from `install_requires`.

0.1.1
-----

Fixes a type error in the python client occuring then checking the timeout for a pending lock if the timeout is set to `None`.

0.1.0
-----

Initial Release
