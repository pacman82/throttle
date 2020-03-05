use crate::{
    application_cfg::Semaphores,
    leases::{Counts, Leases},
};
use lazy_static::lazy_static;
use log::{debug, warn};
use prometheus::IntGaugeVec;
use std::{
    collections::HashMap,
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
    semaphores: Semaphores,
}

#[derive(Debug)]
pub enum Error {
    UnknownSemaphore,
    UnknownLease,
    ForeverPending { asked: i64, max: i64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::UnknownSemaphore => write!(f, "Unknown semaphore"),
            Error::UnknownLease => write!(f, "Unknown lease"),
            Error::ForeverPending { asked, max } => write!(
                f,
                "Acquiring lock would block forever. Lock askes for count {} yet full count is \
                only {}.",
                asked, max
            ),
        }
    }
}

impl State {
    /// Creates the state required for the semaphore service
    pub fn new(semaphores: Semaphores) -> State {
        State {
            leases: Mutex::new(Leases::new()),
            released: Condvar::new(),
            semaphores,
        }
    }

    pub fn acquire(
        &self,
        semaphore: &str,
        amount: u32,
        expires_in: Duration,
    ) -> Result<(u64, bool), Error> {
        if let Some(&max) = self.semaphores.get(semaphore) {
            let mut leases = self.leases.lock().unwrap();
            let valid_until = Instant::now() + expires_in;
            // Return early if lease could never be active, no matter how long we wait
            if max < amount as i64 {
                return Err(Error::ForeverPending {
                    asked: amount as i64,
                    max,
                });
            }
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
        expires_in: Duration,
        semaphore: &str,
        amount: u32,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let mut leases = self.leases.lock().unwrap();
        let start = Instant::now();
        let valid_until = start + expires_in;
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

    pub fn update(&self, lease_id: u64, semaphore: &str, amount: u32, expires_in: Duration) {
        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + expires_in;
        leases.update(lease_id, semaphore, amount, true, valid_until);
    }

    pub fn remainder(&self, semaphore: &str) -> Result<i64, Error> {
        if let Some(full_count) = self.semaphores.get(semaphore) {
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

    /// Update the registered prometheus metrics with values reflecting the current state.State
    ///
    /// This method updates the global default prometheus regestry.
    pub fn update_metrics(&self) {
        let mut counts = HashMap::new();
        for (semaphore, &full_count) in &self.semaphores {
            // Ok, currently we don't support changing the full_count at runtime, but let's keep it
            // here for later use.
            FULL_COUNT.with_label_values(&[semaphore]).set(full_count);
            // Doing all these nasty allocations before acquiring the lock to leases
            counts.insert(semaphore.clone(), Counts::default());
        }
        // Most of the work happens in here. Now counts contains the active and pending counts
        self.leases.lock().unwrap().fill_counts(&mut counts);
        for (semaphore, count) in counts {
            COUNT.with_label_values(&[&semaphore]).set(count.active);
            PENDING.with_label_values(&[&semaphore]).set(count.pending);
        }
    }
}

lazy_static! {
    static ref FULL_COUNT: IntGaugeVec =
        register_int_gauge_vec!(
            "throttle_full_count",
            "New leases which would increase the count beyond this limit are pending.",
            &["semaphore"]
        ).expect("Error registering throttle_full_count metric");

    static ref COUNT: IntGaugeVec =
        register_int_gauge_vec!(
            "throttle_count",
            "Accumulated count of all active leases",
            &["semaphore"]
        ).expect("Error registering throttle_count metric");

    static ref PENDING: IntGaugeVec =
        register_int_gauge_vec!(
            "throttle_pending",
            "Accumulated count of all pending leases",
            &["semaphore"]
        ).expect("Error registering throttle_count metric");
}
