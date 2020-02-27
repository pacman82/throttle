use crate::{application_cfg::ApplicationCfg, leases::Leases};
use log::{debug, warn};
use std::{
    fmt,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// State of the Semaphore service, shared between threads
pub struct State {
    /// Bookeeping for leases, protected by mutex.
    leases: Mutex<Leases>,
    /// Condition variable. Notify is called thenever a lease is released, so it's suitable for
    /// blocking on request to pending leases.
    released: Condvar,
    /// All known semaphores and their full count
    cfg: ApplicationCfg,
}

#[derive(Debug)]
pub enum Error {
    UnknownSemaphore,
    UnknownLease,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::UnknownSemaphore => write!(f, "Unknown semaphore"),
            Error::UnknownLease => write!(f, "Unknown lease"),
        }
    }
}

impl State {
    /// Creates the state required for the semaphore service
    pub fn new(cfg: ApplicationCfg) -> State {
        State {
            leases: Mutex::new(Leases::new()),
            released: Condvar::new(),
            cfg,
        }
    }

    pub fn acquire(
        &self,
        semaphore: &str,
        amount: u32,
        valid_for: Duration,
    ) -> Result<(u64, bool), Error> {
        if let Some(&max) = self.cfg.semaphores.get(semaphore) {
            let mut leases = self.leases.lock().unwrap();
            let valid_until = Instant::now() + valid_for;
            let (active, lease_id) = leases.add(semaphore, amount, max, valid_until);
            if active {
                debug!("Lease {} to '{}' acquired.", lease_id, semaphore);
                Ok((lease_id, true))
            } else {
                debug!("Ticket {} for '{}' created.", lease_id, semaphore);
                Ok((lease_id, false))
            }
        } else {
            warn!("Unknown semaphore '{}' requested", semaphore);
            Err(Error::UnknownSemaphore)
        }
    }

    /// Removes leases outdated due to timestamp. Wakes threads waiting for pending leases if any
    /// leases are removed.
    ///
    /// Returns number of (now removed) expired leases
    pub fn remove_expired(&self) -> usize {
        let num_removed = self.leases.lock().unwrap().remove_expired(Instant::now());
        if num_removed != 0 {
            self.released.notify_all();
            warn!("Removed {} leases due to expiration.", num_removed);
        }
        num_removed
    }

    pub fn wait_for_admission(
        &self,
        lease_id: u64,
        valid_for: Duration,
        semaphore: &str,
        amount: u32,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let mut leases = self.leases.lock().unwrap();
        let start = Instant::now();
        let valid_until = start + valid_for;
        leases.update(lease_id, semaphore, amount, false, valid_until);
        loop {
            break match leases.has_pending(lease_id) {
                None => {
                    warn!(
                        "Unknown lease accessed while waiting for admission request. Lease id: {}",
                        lease_id
                    );
                    Err(Error::UnknownLease)
                }
                Some(true) => Ok(true),
                Some(false) => {
                    let elapsed = start.elapsed(); // Uses a monotonic system clock
                    if elapsed >= timeout {
                        // Lease is pending, even after timeout is passed
                        Ok(false)
                    } else {
                        // Lease is pending, but timeout hasn't passed yet. Let's wait for changes.
                        let (mutex_guard, wait_time_result) = self
                            .released
                            .wait_timeout(leases, timeout - elapsed)
                            .unwrap();
                        if wait_time_result.timed_out() {
                            Ok(false)
                        } else {
                            leases = mutex_guard;
                            continue;
                        }
                    }
                }
            };
        }
    }

    pub fn update(&self, lease_id: u64, semaphore: &str, amount: u32, valid_for: Duration) {
        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + valid_for;
        leases.update(lease_id, semaphore, amount, true, valid_until);
    }

    pub fn remainder(&self, semaphore: &str) -> Result<i64, Error> {
        if let Some(full_count) = self.cfg.semaphores.get(semaphore) {
            let leases = self.leases.lock().unwrap();
            let count = leases.count(&semaphore);
            Ok(full_count - count)
        } else {
            warn!("Unknown semaphore requested");
            Err(Error::UnknownSemaphore)
        }
    }

    /// Releases a previously acquired lease.
    ///
    /// Returns `false` should the lease not be found and `true` otherwise. `false` could occur due
    /// to e.g. the `lease` already being removed by litter collection.
    pub fn release(&self, lease_id: u64) -> bool {
        let mut leases = self.leases.lock().unwrap();
        match leases.remove(lease_id) {
            Some(semaphore) => {
                let full_count = self
                    .cfg
                    .semaphores
                    .get(&semaphore)
                    .expect("An active semaphore must always be configured");
                leases.resolve_pending(&semaphore, *full_count);
                // Notify waiting requests that lease has changed
                self.released.notify_all();
                true
            }
            None => {
                warn!("Deletion of unknown lease.");
                false
            }
        }
    }
}
