use crate::{
    application_cfg::Semaphores,
    error::ThrottleError,
    leases::{Counts, Leases, PeerId},
    wakers::Wakers,
};
use lazy_static::lazy_static;
use log::{debug, warn};
use prometheus::IntGaugeVec;
use std::{
    collections::HashMap,
    mem::drop,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::time;

/// State of the Semaphore service, shared between threads
///
/// This class combines the confguration of the `semaphores`, with the state of the peers in
/// `leases` and makes them consumable in an asynchronous, mulithreaded consumer.
pub struct State {
    /// All known semaphores and their full count
    semaphores: Semaphores,
    /// Bookeeping for leases, protected by mutex so multiple threads (i.e. requests) can manipulate
    /// it. Must not contain any leases not configured in semaphores.
    leases: Mutex<Leases>,
    /// Peer id and weak references to mutex for each pending request.
    wakers: Wakers,
}

impl State {
    /// Creates the state required for the semaphore service
    pub fn new(semaphores: Semaphores) -> State {
        State {
            leases: Mutex::new(Leases::new()),
            semaphores,
            wakers: Wakers::new(),
        }
    }

    /// Creates a new peer.
    pub fn new_peer(&self, expires_in: Duration) -> PeerId {
        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + expires_in;
        let peer_id = leases.new_peer(valid_until);
        debug!("Created new peer {}.", peer_id);
        peer_id
    }

    /// Sets the lock count for this peer and semaphore to `amount`. Should the remainder of the
    /// semaphore allow it.
    ///
    /// * `peer_id`: Identifies the peer for which we are setting the lock count
    /// * `semaphore`: Name of the semaphore to which we want to acquire the lock count.
    /// * `amount`: The count of the lock. Outside of revenants (i.e. expired peers, which do
    /// return). Throttle is goingt to see to it that the combined lock count is not going beyond
    /// the configured max count.
    /// * `wait_for`: The future returns as soon as the lock could be acquireod or after the
    /// duration has elapsed, even if the lock could not be acquired. If set to `None`, the future
    /// returns immediatly.
    /// * `expires_in`: Used to prolong the expiration timestamp of the affected peer. This saves us
    /// an extra heartbeat request.
    pub async fn acquire(
        &self,
        peer_id: PeerId,
        semaphore: &str,
        amount: u32,
        wait_for: Option<Duration>,
        expires_in: Duration,
    ) -> Result<bool, ThrottleError> {
        let max = *self
            .semaphores
            .get(semaphore)
            .ok_or(ThrottleError::UnknownSemaphore)?;
        // Return early if lease can never be acquired
        if max < amount as i64 {
            return Err(ThrottleError::ForeverPending {
                asked: amount as i64,
                max,
            });
        }
        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + expires_in;
        leases.update_valid_until(peer_id, valid_until)?;
        let acquired = leases.acquire(peer_id, semaphore, amount, Some(max))?;
        if acquired {
            // Resolve this immediatly, if we can
            debug!("Peer {} acquired lock to '{}'.", peer_id, semaphore);
            Ok(true)
        } else {
            debug!(
                "Peer {} waiting for lock to '{}'",
                peer_id, semaphore
            );

            // We could not acquire the lock immediatly. Are we going to wait for it?
            if let Some(wait_for) = wait_for {
                // keep holding the lock to `leases` until everything is registered. So we don't miss
                // the call to `resolve_with`.

                let acquire_or_timeout =
                    time::timeout(wait_for, self.wakers.wait_for_resolving(peer_id));

                // Release lock on leases, while waiting for acquire_or_timeout! Otherwise, we might
                // deadlock.
                drop(leases);
                // The outer `Err` indicates a timeout.
                match acquire_or_timeout.await {
                    // Locks could be acquired
                    Ok(Ok(())) => {
                        debug!("Peer {} acquired lock to '{}'.", peer_id, semaphore);
                        Ok(true)
                    }
                    // Failure
                    Ok(Err(e)) => Err(e),
                    // Lock could not be acquired in time
                    Err(_) => Ok(false),
                }
            } else {
                Ok(acquired)
            }
        }
    }

    /// Removes leases outdated due to timestamp. Wakes threads waiting for pending leases if any
    /// leases are removed.
    ///
    /// Returns number of (now removed) expired leases
    pub fn remove_expired(&self) -> usize {
        let (expired_peers, resolved_peers) = {
            let mut leases = self.leases.lock().unwrap();
            let (expired_peers, affected_semaphores) = leases.remove_expired(Instant::now());
            let mut resolved_peers = Vec::new();
            for semaphore in affected_semaphores {
                resolved_peers.extend(
                    leases.resolve_pending(&semaphore, *self.semaphores.get(&semaphore).unwrap()),
                );
            }
            (expired_peers, resolved_peers)
        };
        if !expired_peers.is_empty() {
            warn!("Removed {} peers due to expiration.", expired_peers.len());
            // It is not enough to notify only the requests for the removed peers, as other peers
            // might be able to acquire their locks due to the removal of these.
            self.wakers.resolve_with(&resolved_peers, Ok(()));
            self.wakers
                .resolve_with(&expired_peers, Err(ThrottleError::UnknownPeer));
        }
        expired_peers.len()
    }

    /// Restore peer
    pub fn restore(
        &self,
        peer_id: PeerId,
        expires_in: Duration,
        pending: Option<(&str, u32)>,
        acquired: Option<(&str, u32)>,
    ) -> Result<bool, ThrottleError> {
        let pending = pending;
        let acquired = acquired;

        if let Some((semaphore, count)) = pending.or(acquired) {

            // Assert semaphore exists. We want to give the client an error and also do not want to
            // allow any Unknown Semaphore into `leases`.
            let max = *self
                .semaphores
                .get(semaphore)
                .ok_or(ThrottleError::UnknownSemaphore)?;
                
            let mut leases = self.leases.lock().unwrap();
            let valid_until = Instant::now() + expires_in;
            leases.new_peer_at(peer_id, valid_until);
            let max = if acquired.is_some() {
                // If the restored lease has the lock already acquired, there is no point in checking it
                // against the full semaphore count. The resource the semaphore is protecting is already
                // being accessed by it. Better to count it as acquired anyway, even if we increment our
                // active semaphore count beyond the full count.
                //
                // By passing None as max rather than the value obtained above, we opt out checking the
                // semaphore full count and allow exceeding it.
                None
            } else {
                Some(max)
            };
            let acquired = leases.acquire(peer_id, semaphore, count, max)?;
            warn!("Revenant Peer {}.", peer_id);

            Ok(acquired)
        } else {
            let mut leases = self.leases.lock().unwrap();
            let valid_until = Instant::now() + expires_in;
            leases.new_peer_at(peer_id, valid_until);
            Ok(true)
        }
    }

    pub fn heartbeat(&self, peer_id: PeerId, expires_in: Duration) -> Result<(), ThrottleError> {
        let mut leases = self.leases.lock().unwrap();
        // Determine valid_until after acquiring lock, in case we block for a long time.
        let valid_until = Instant::now() + expires_in;
        leases.update_valid_until(peer_id, valid_until)?;
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
    pub fn release(&self, peer_id: PeerId) -> bool {
        let mut leases = self.leases.lock().unwrap();
        match leases.remove(peer_id) {
            Some(semaphore) => {
                let full_count = self
                    .semaphores
                    .get(&semaphore)
                    .expect("An active semaphore must always be configured");
                let resolved_peers = leases.resolve_pending(&semaphore, *full_count);
                drop(leases); // Don't hold this longer than we need to.
                self.wakers.resolve_with(&resolved_peers, Ok(()));
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

    /// Returns true if all the locks of the peer are acquired
    pub fn is_acquired(&self, peer_id: PeerId) -> Result<bool, ThrottleError> {
        let leases = self.leases.lock().unwrap();
        leases.has_pending(peer_id).map(|pending| !pending)
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
    use tokio;

    #[tokio::test]
    async fn acquire_three_leases() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), 3);
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // First three locks can be acquired immediatly
        let one = state.new_peer(one_sec);
        assert!(state.acquire(one, "A", 1, None, one_sec).await.unwrap());
        let two = state.new_peer(one_sec);
        assert!(state.acquire(two, "A", 1, None, one_sec).await.unwrap());
        let three = state.new_peer(one_sec);
        assert!(state.acquire(three, "A", 1, None, one_sec).await.unwrap());
        // The fourth must wait
        let four = state.new_peer(one_sec);
        assert!(!state.acquire(four, "A", 1, None, one_sec).await.unwrap());
    }

    #[tokio::test]
    async fn resolve_pending() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), 3);
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // Create six peers
        let p: Vec<_> = (0..6).map(|_| state.new_peer(one_sec)).collect();

        // First three locks can be acquired immediatly
        state.acquire(p[0], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[1], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[2], "A", 1, None, one_sec).await.unwrap();
        // The four, five and six must wait
        state.acquire(p[3], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[4], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[5], "A", 1, None, one_sec).await.unwrap();
        // Remainder is zero due to the three leases intially acquired
        assert_eq!(state.remainder("A").unwrap(), 0);
        // Release one of the first three. Four should now be acquired.
        state.release(p[1]);
        assert_eq!(state.remainder("A").unwrap(), 0);

        // Release another one of the first three. Five should now be acquired.
        state.release(p[0]);
        assert_eq!(state.remainder("A").unwrap(), 0);

        // Release last one of the first three. six should now be acquired.
        state.release(p[2]);
        assert_eq!(state.remainder("A").unwrap(), 0);
    }

    /// Pending locks must be acquired in the same order the have been requested.
    #[tokio::test]
    async fn fairness() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), 3);
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // Create six peers
        let p: Vec<_> = (0..6).map(|_| state.new_peer(one_sec)).collect();

        // First three locks can be acquired immediatly
        state.acquire(p[0], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[1], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[2], "A", 1, None, one_sec).await.unwrap();
        // The four, five and six must wait
        state.acquire(p[3], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[4], "A", 1, None, one_sec).await.unwrap();
        state.acquire(p[5], "A", 1, None, one_sec).await.unwrap();
        // Remainder is zero due to the three leases intially acquired
        assert_eq!(state.remainder("A").unwrap(), 0);
        // Release one of the first three. Four should now be acquired.
        state.release(p[1]);
        assert!(state.is_acquired(p[3]).unwrap());

        // Release another one of the first three. Five should now be acquired.
        state.release(p[0]);
        assert!(state.is_acquired(p[4]).unwrap());

        // Release last one of the first three. six should now be acquired.
        state.release(p[2]);
        assert!(state.is_acquired(p[5]).unwrap());
    }

    #[tokio::test]
    async fn idempotent_acquire() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), 1);
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let first = state.new_peer(one_sec);
        assert!(state.acquire(first, "A", 1, None, one_sec).await.unwrap());
        assert!(state.acquire(first, "A", 1, None, one_sec).await.unwrap());

        let second = state.new_peer(one_sec);
        assert!(!state.acquire(second, "A", 1, None, one_sec).await.unwrap());
        assert!(!state.acquire(second, "A", 1, None, one_sec).await.unwrap());
    }
}
