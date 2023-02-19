use crate::{
    application_cfg::Semaphores,
    error::ThrottleError,
    leases::{Counts, Leases, PeerId},
};
use async_events::AsyncEvents;
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
    wakers: AsyncEvents<u64, Result<(), ThrottleError>>,
}

impl State {
    /// Creates the state required for the semaphore service
    pub fn new(semaphores: Semaphores) -> State {
        State {
            leases: Mutex::new(Leases::new()),
            semaphores,
            wakers: AsyncEvents::new(),
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
        amount: i64,
        wait_for: Option<Duration>,
        expires_in: Option<Duration>,
    ) -> Result<bool, ThrottleError> {
        let sem = self.semaphores.get(semaphore).ok_or_else(|| {
            warn!("Unknown semaphore requested: {}", semaphore);
            ThrottleError::UnknownSemaphore
        })?;
        let max = sem.max;
        let level = sem.level;
        // Return early if lease can never be acquired
        if max < amount {
            return Err(ThrottleError::Never { asked: amount, max });
        }
        let (acquired, acquire_or_timeout) = {
            let mut leases = self.leases.lock().unwrap();
            if let Some(expires_in) = expires_in {
                let valid_until = Instant::now() + expires_in;
                leases.update_valid_until(peer_id, valid_until)?;
            }
            let acquired = leases.acquire(peer_id, semaphore, amount, max, level, |s| {
                self.semaphores.get(s).unwrap().level
            })?;
            if acquired {
                // Resolve this immediatly
                debug!("Peer {} acquired lock to '{}'.", peer_id, semaphore);
                (true, None)
            } else {
                debug!("Peer {} waiting for lock to '{}'", peer_id, semaphore);
                let acquire_or_timeout = wait_for.map(|wait_for| {
                    // We could not acquire the lock immediatly. Are we going to wait for it?
                    time::timeout(wait_for, self.wakers.output_of(peer_id))
                });
                (false, acquire_or_timeout)
            }
            // Release lock on leases, before waiting for acquire_or_timeout! Otherwise, we might
            // deadlock.
        };
        
        if let Some(acquire_or_timeout) = acquire_or_timeout {
            // We could not acquire lock right away, and user decided to wait a bit, so this is what
            // we do.

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
            // We could acquire the lock immediatly, or the user did not decide to wait. Either way
            // we can answer right away.
            Ok(acquired)
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
            // It is not enough to notify only the requests for the removed peers, as other peers
            // might be able to acquire their locks due to the removal of these.
            let mut resolved_peers = Vec::new();
            for semaphore in affected_semaphores {
                leases.resolve_pending(
                    &semaphore,
                    self.semaphores.get(&semaphore).unwrap().max,
                    &mut resolved_peers,
                )
            }
            (expired_peers, resolved_peers)
        };
        if !expired_peers.is_empty() {
            warn!("Removed {} peers due to expiration.", expired_peers.len());
            self.wakers.resolve_all_with(&resolved_peers, Ok(()));
            self.wakers
                .resolve_all_with(&expired_peers, Err(ThrottleError::UnknownPeer));
        } else {
            debug!("Litter collection found no expired leases.");
        }
        expired_peers.len()
    }

    /// Restore peer
    pub fn restore(
        &self,
        peer_id: PeerId,
        expires_in: Duration,
        acquired: &HashMap<String, i64>,
    ) -> Result<(), ThrottleError> {
        warn!(
            "Revenant Peer {}. Has locks: {}",
            peer_id,
            !acquired.is_empty()
        );

        for semaphore in acquired.keys() {
            // Assert semaphore exists. We want to give the client an error and also do not want to
            // allow any Unknown Semaphore into `leases`. Also we want to fail fast, before
            // acquiring the lock to `leases`.
            let _max = *self
                .semaphores
                .get(semaphore)
                .ok_or(ThrottleError::UnknownSemaphore)?;
        }

        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + expires_in;

        // Acquired all locks for the peer
        leases.restore(peer_id, acquired, valid_until)?;

        Ok(())
    }

    pub fn heartbeat(&self, peer_id: PeerId, expires_in: Duration) -> Result<(), ThrottleError> {
        let mut leases = self.leases.lock().unwrap();
        // Determine valid_until after acquiring lock, in case we block for a long time.
        let valid_until = Instant::now() + expires_in;
        leases.update_valid_until(peer_id, valid_until)?;
        Ok(())
    }

    pub fn remainder(&self, semaphore: &str) -> Result<i64, ThrottleError> {
        if let Some(sem) = self.semaphores.get(semaphore) {
            let leases = self.leases.lock().unwrap();
            let count = leases.count(semaphore);
            Ok(sem.max - count)
        } else {
            warn!("Unknown semaphore requested: {}", semaphore);
            Err(ThrottleError::UnknownSemaphore)
        }
    }

    /// Removes a peer from bookeeping and releases all acquired leases.
    ///
    /// Returns `false` should the peer not be found and `true` otherwise. `false` could occur due
    /// to e.g. the peer already being removed by litter collection.
    pub fn release(&self, peer_id: PeerId) -> bool {
        let mut leases = self.leases.lock().unwrap();
        match leases.remove_peer(peer_id) {
            Some(semaphores) => {
                // Keep book about all peers, those locks have been acquired, so we can notify their pending
                // requests.
                let mut resolved_peers = Vec::new();
                for semaphore in semaphores {
                    let sem = self
                        .semaphores
                        .get(&semaphore)
                        .expect("An active semaphore must always be configured");
                    leases.resolve_pending(&semaphore, sem.max, &mut resolved_peers);
                }
                drop(leases); // Don't hold this longer than we need to.
                self.wakers.resolve_all_with(&resolved_peers, Ok(()));
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
        for (name, &sem) in &self.semaphores {
            // Ok, currently we don't support changing the full_count at runtime, but let's keep it
            // here for later use.
            FULL_COUNT.with_label_values(&[name]).set(sem.max);
            // Doing all these nasty allocations before acquiring the lock to leases
            counts.insert(name.clone(), Counts::default());
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

    /// Releases a lock associated with the peer. Due to the relased lock, other locks may be
    /// acquired, futures may need to be woken.
    pub fn release_lock(&self, peer_id: PeerId, semaphore: &str) -> Result<(), ThrottleError> {
        let max = self
            .semaphores
            .get(semaphore)
            .ok_or(ThrottleError::UnknownSemaphore)?
            .max;
        let mut leases = self.leases.lock().unwrap();
        leases.release_lock(peer_id, semaphore)?;
        let mut resolved_peers = Vec::new();
        leases.resolve_pending(semaphore, max, &mut resolved_peers);
        self.wakers.resolve_all_with(&resolved_peers, Ok(()));
        Ok(())
    }
}

lazy_static! {
    static ref FULL_COUNT: IntGaugeVec = register_int_gauge_vec!(
        "throttle_max",
        "Maximum allowed lock count for this semaphore.",
        &["semaphore"]
    )
    .expect("Error registering throttle_full_count metric");
    static ref COUNT: IntGaugeVec = register_int_gauge_vec!(
        "throttle_acquired",
        "Sum of all acquired locks.",
        &["semaphore"]
    )
    .expect("Error registering throttle_count metric");
    static ref PENDING: IntGaugeVec = register_int_gauge_vec!(
        "throttle_pending",
        "Sum of all pending locks",
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
    use crate::application_cfg::SemaphoreCfg;

    #[tokio::test]
    async fn acquire_three_leases() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 3, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // First three locks can be acquired immediatly
        let one = state.new_peer(one_sec);
        assert!(state.acquire(one, "A", 1, None, None).await.unwrap());
        let two = state.new_peer(one_sec);
        assert!(state.acquire(two, "A", 1, None, None).await.unwrap());
        let three = state.new_peer(one_sec);
        assert!(state.acquire(three, "A", 1, None, None).await.unwrap());
        // The fourth must wait
        let four = state.new_peer(one_sec);
        assert!(!state.acquire(four, "A", 1, None, None).await.unwrap());
    }

    #[tokio::test]
    async fn resolve_pending() {
        // Semaphore with count of 3
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 3, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // Create six peers
        let p: Vec<_> = (0..6).map(|_| state.new_peer(one_sec)).collect();

        // First three locks can be acquired immediatly
        state.acquire(p[0], "A", 1, None, None).await.unwrap();
        state.acquire(p[1], "A", 1, None, None).await.unwrap();
        state.acquire(p[2], "A", 1, None, None).await.unwrap();
        // The four, five and six must wait
        state.acquire(p[3], "A", 1, None, None).await.unwrap();
        state.acquire(p[4], "A", 1, None, None).await.unwrap();
        state.acquire(p[5], "A", 1, None, None).await.unwrap();
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
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 3, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // Create six peers
        let p: Vec<_> = (0..6).map(|_| state.new_peer(one_sec)).collect();

        // First three locks can be acquired immediatly
        state.acquire(p[0], "A", 1, None, None).await.unwrap();
        state.acquire(p[1], "A", 1, None, None).await.unwrap();
        state.acquire(p[2], "A", 1, None, None).await.unwrap();
        // The four, five and six must wait
        state.acquire(p[3], "A", 1, None, None).await.unwrap();
        state.acquire(p[4], "A", 1, None, None).await.unwrap();
        state.acquire(p[5], "A", 1, None, None).await.unwrap();
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
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let first = state.new_peer(one_sec);
        assert!(state.acquire(first, "A", 1, None, None).await.unwrap());
        assert!(state.acquire(first, "A", 1, None, None).await.unwrap());

        let second = state.new_peer(one_sec);
        assert!(!state.acquire(second, "A", 1, None, None).await.unwrap());
        assert!(!state.acquire(second, "A", 1, None, None).await.unwrap());
    }

    #[tokio::test]
    async fn multiple_locks_per_peer() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 2, level: 1 });
        semaphores.insert(String::from("B"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let first = state.new_peer(one_sec);
        // Acquire one of 'A' and 'B' each.
        assert!(state.acquire(first, "A", 1, None, None).await.unwrap());
        assert!(state.acquire(first, "B", 1, None, None).await.unwrap());

        let second = state.new_peer(one_sec);
        // Second can still acquire lock to 'A' since its full count is 2, but 'B' must pend.
        assert!(state.acquire(second, "A", 1, None, None).await.unwrap());
        assert!(!state.acquire(second, "B", 1, None, None).await.unwrap());

        state.release(first);
        assert!(state.is_acquired(second).unwrap());
    }

    /// Provoke two pending locks for the same peer.
    #[tokio::test]
    async fn two_pending_locks() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 1 });
        semaphores.insert(String::from("B"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        // Acquire both semaphores with blocker, so all other locks are going to be pending.
        let blocker = state.new_peer(one_sec);
        state.acquire(blocker, "A", 1, None, None).await.unwrap();
        state.acquire(blocker, "B", 1, None, None).await.unwrap();

        let peer = state.new_peer(one_sec);
        assert!(!state.acquire(peer, "A", 1, None, None).await.unwrap());
        assert!(matches!(
            state.acquire(peer, "B", 1, None, None).await,
            Err(ThrottleError::AlreadyPending)
        ));
    }

    #[tokio::test]
    async fn acquire_zero() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let peer = state.new_peer(one_sec);
        assert!(matches!(
            state.acquire(peer, "A", 0, None, None).await,
            Err(ThrottleError::InvalidLockCount { count: 0 })
        ));
    }

    #[tokio::test]
    async fn restore_with_lock_count_zero() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let peer = 5;
        let mut acquired = HashMap::new();
        acquired.insert(String::from("A"), 0);
        assert!(matches!(
            state.restore(peer, one_sec, &acquired),
            Err(ThrottleError::InvalidLockCount { count: 0 })
        ));
    }

    #[tokio::test]
    async fn restore_with_unknown_semaphore() {
        let state = State::new(Semaphores::new());
        let one_sec = Duration::from_secs(1);

        let peer = 5;
        let mut acquired = HashMap::new();
        acquired.insert(String::from("A"), 1);
        assert!(matches!(
            state.restore(peer, one_sec, &acquired),
            Err(ThrottleError::UnknownSemaphore)
        ));
    }

    #[tokio::test]
    async fn restore_cant_change_existing_peers() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let peer = state.new_peer(one_sec);

        let mut acquired = HashMap::new();
        acquired.insert(String::from("A"), 1);
        assert!(matches!(
            state.restore(peer, one_sec, &acquired),
            Err(ThrottleError::ChangeThroughRestore)
        ));
    }

    #[tokio::test]
    async fn enforce_lock_hierachies() {
        let mut semaphores = Semaphores::new();
        semaphores.insert(String::from("A"), SemaphoreCfg { max: 1, level: 1 });
        semaphores.insert(String::from("B"), SemaphoreCfg { max: 1, level: 0 });
        let state = State::new(semaphores);
        let one_sec = Duration::from_secs(1);

        let first = state.new_peer(one_sec);
        // Try acquiring in wrong order. First B then A.
        state.acquire(first, "B", 1, None, None).await.unwrap();
        // This should result in a lock hierachie violation
        assert!(matches!(
            state.acquire(first, "A", 1, None, None).await,
            Err(ThrottleError::Deadlock {
                current: 0,
                requested: 1
            })
        ));
    }
}
