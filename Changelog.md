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

0.1.4
-----

* Python Client: Use tenacity for all requests

0.1.5
-----

* Python Client: `Client` can now be pickeled again.

0.1.6
-----

* Fairness
* Large semaphores don't starve

0.1.7
-----

* Fix: 0.1.6 introduced a behaviour, there acquiring a lock would always fail initialy, for all but
  the first peer.

0.1.8
-----

* Fix: Releasing locks, did fail to stop other locks from pending if a higher priority lock had
already been acquired.
* Favicon route is now `favicon.ico` instead of `favicon`.
* Add route `version` to display current version number.

0.1.9
-----

* Fix: Pending leases are now acquired immediatly. Previously their acquiration could have been
delayed if the peer holding the lock previously did expire, rather than release its lock.
* New route `is_acquired` tells if all the locks of a peer could be acquired.

0.2.0 Next
----------

* Removed HTTP route `/freeze`.
* Recover from unknown peer is now handled on the client side.
  * Uses new route `/restore`.
* Removed `Peer.has_pending()`.
* Acquiring locks is now idempotent
