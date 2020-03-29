use crate::{
    application_cfg::Semaphores,
    error::ThrottleError,
    leases::{Counts, Leases},
};
use lazy_static::lazy_static;
use log::{debug, warn};
use prometheus::IntGaugeVec;
use std::{
    collections::HashMap,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// State of the Semaphore service, shared between threads
pub struct State {
    /// Bookeeping for leases, protected by mutex so multiple threads (i.e. requests) can manipulate
    /// it. Must not contain any leases not configured in semaphores.
    leases: Mutex<Leases>,
    /// Condition variable. Notify is called thenever a lease is released, so it's suitable for
    /// blocking on request to pending leases.
    released: Condvar,
    /// All known semaphores and their full count
    semaphores: Semaphores,
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
    ) -> Result<(u64, bool), ThrottleError> {
        if let Some(&max) = self.semaphores.get(semaphore) {
            // Return early if lease could never be active, no matter how long we wait
            if max < amount as i64 {
                return Err(ThrottleError::ForeverPending {
                    asked: amount as i64,
                    max,
                });
            }
            let mut leases = self.leases.lock().unwrap();
            let valid_until = Instant::now() + expires_in;
            // This is a new peer. Let's make a new `peer_id` to remember it by.
            let peer_id = leases.new_unique_peer_id();
            let active = leases.add(peer_id, semaphore, amount, Some(max), valid_until);
            if active {
                debug!("Peer {} acquired lease to '{}'.", peer_id, semaphore);
                Ok((peer_id, true))
            } else {
                debug!("Peer {} waiting for lease to '{}'.", peer_id, semaphore);
                Ok((peer_id, false))
            }
        } else {
            warn!("Unknown semaphore '{}' requested", semaphore);
            Err(ThrottleError::UnknownSemaphore)
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

    /// Blocks until all the leases of the peer are active, or the timeout expires.
    ///
    /// Intended te be called repeatedly, until leases are active. Also prevents the peer from being
    /// removed by the litter collection, as it updates the expiration timestamp.
    ///
    /// ## Return
    ///
    /// Returns `true` if the the leases could be acquired.
    pub fn block_until_acquired(
        &self,
        peer_id: u64,
        expires_in: Duration,
        semaphore: &str,
        amount: u32,
        timeout: Duration,
    ) -> Result<bool, ThrottleError> {
        let mut leases = self.leases.lock().unwrap();
        let start = Instant::now();
        let valid_until = start + expires_in;
        if !leases.update_valid_until(peer_id, valid_until) {
            let max = *self
                .semaphores
                .get(semaphore)
                .ok_or(ThrottleError::UnknownSemaphore)?;
            let active = leases.add(peer_id, semaphore, amount, Some(max), valid_until);
            warn!(
                "Revenant Peer {} with pending leases. Active: {}",
                peer_id, active
            );
        }

        let (leases, wait_time_result) = self
            .released
            .wait_timeout_while(leases, timeout, |leases| {
                leases.has_pending(peer_id).unwrap_or(false)
            })
            .unwrap();

        // Can have error if a peer expires while waitinng for a lease.
        let pending = leases
            .has_pending(peer_id)
            .ok_or(ThrottleError::UnknownPeer)?;
        debug_assert!(pending == wait_time_result.timed_out());
        Ok(!pending)
    }

    pub fn heartbeat_for_active_peer(
        &self,
        peer_id: u64,
        semaphore: &str,
        amount: u32,
        expires_in: Duration,
    ) -> Result<(), ThrottleError> {
        let mut leases = self.leases.lock().unwrap();
        // Determine valid_until after acquiring lock, in case we block for a long time.
        let valid_until = Instant::now() + expires_in;
        if !leases.update_valid_until(peer_id, valid_until) {
            // Assert semaphore exists. We want to give the client an error and also do not want to
            // allow any Unknown Semaphore into `leases`.
            let _max = *self
                .semaphores
                .get(semaphore)
                .ok_or(ThrottleError::UnknownSemaphore)?;
            // By passing None as max rather than the value obtained above, we opt out checking the
            // semaphore full count and allow exceeding it.
            let max = None;
            leases.add(peer_id, semaphore, amount, max, valid_until);
            warn!("Revenat peer {} with active leases returned.", peer_id);
        }
        Ok(())
    }

    pub fn remainder(&self, semaphore: &str) -> Result<i64, ThrottleError> {
        if let Some(full_count) = self.semaphores.get(semaphore) {
            let leases = self.leases.lock().unwrap();
            let count = leases.count(&semaphore);
            Ok(full_count - count)
        } else {
            warn!("Unknown semaphore requested");
            Err(ThrottleError::UnknownSemaphore)
        }
    }

    /// Removes a peer from bookeeping and releases all acquired leases.
    ///
    /// Returns `false` should the peer not be found and `true` otherwise. `false` could occur due
    /// to e.g. the peer already being removed by litter collection.
    pub fn release(&self, peer_id: u64) -> bool {
        let mut leases = self.leases.lock().unwrap();
        match leases.remove(peer_id) {
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
                warn!("Deletion of unknown peer.");
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
        let now = Instant::now();
        for (semaphore, count) in counts {
            COUNT.with_label_values(&[&semaphore]).set(count.acquired);
            PENDING.with_label_values(&[&semaphore]).set(count.pending);
            LONGEST_PENDING_SEC
                .with_label_values(&[&semaphore])
                .set(count.longest_pending(now).as_secs() as i64)
        }
    }

    /// Lock internal mutex for specified amount of time, and thus preventing any changes. No use in
    /// production, but useful during testing to emulate scenarios there the server is stressed and
    /// to provoke timeouts.
    pub fn freeze(&self, time: Duration) {
        let _lock = self.leases.lock();
        warn!("Frozen for {} seconds", time.as_secs_f64());
        std::thread::sleep(time);
        warn!("Unfrozen");
    }
}

lazy_static! {
    static ref FULL_COUNT: IntGaugeVec = register_int_gauge_vec!(
        "throttle_full_count",
        "New leases which would increase the count beyond this limit are pending.",
        &["semaphore"]
    )
    .expect("Error registering throttle_full_count metric");
    static ref COUNT: IntGaugeVec = register_int_gauge_vec!(
        "throttle_count",
        "Accumulated count of all active leases",
        &["semaphore"]
    )
    .expect("Error registering throttle_count metric");
    static ref PENDING: IntGaugeVec = register_int_gauge_vec!(
        "throttle_pending",
        "Accumulated count of all pending leases",
        &["semaphore"]
    )
    .expect("Error registering throttle_count metric");
    static ref LONGEST_PENDING_SEC: IntGaugeVec = register_int_gauge_vec!(
        "throttle_longest_pending_sec",
        "Time the longest pending peer is waiting until now, to acquire a lock to a semaphore.",
        &["semaphore"]
    )
    .expect("Error registering throttle_count metric");
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn acquire_three_leases() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), 3);
        let state = State::new(semaphores);

        // First three locks can be acquired immediatly
        assert!(state.acquire("A", 1, Duration::from_secs(1)).unwrap().1);
        assert!(state.acquire("A", 1, Duration::from_secs(1)).unwrap().1);
        assert!(state.acquire("A", 1, Duration::from_secs(1)).unwrap().1);
        // The fourth must wait
        assert!(!state.acquire("A", 1, Duration::from_secs(1)).unwrap().1);
    }
}
