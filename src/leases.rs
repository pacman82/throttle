use crate::error::ThrottleError;
use rand::random;
use std::{
    cmp::Ordering,
    collections::HashMap,
    time::{Duration, Instant},
};

/// Peers hold locks to semaphores, while holding this locks they have access to the semaphore.
struct Lock {
    /// Name of the resource the semaphore protects
    semaphore: String,
    /// The semapohre count is decreased by `count` if the lease is active.
    count: i64,
    /// Instant of lock creation. Used to implement fairness of semaphores.
    since: Instant,
}

impl Lock {
    /// Semaphore count of this peer regardless of wether the lock is acquired or pending.
    fn count(&self, semaphore: &str) -> i64 {
        if self.semaphore == semaphore {
            self.count
        } else {
            0
        }
    }

    /// Increments the suitable entries in `counts`.
    fn update_counts_pending(&self, counts: &mut HashMap<String, Counts>) {
        let mut counts = counts
            .get_mut(&self.semaphore)
            .expect("All available Semaphores must be prefilled in counts.");
        counts.pending += self.count;
        // If there already has been a minimum, compare. Otherwise just use `self.since`.
        counts.longest_pending_since = counts
            .longest_pending_since
            .map(|min_so_far| std::cmp::min(min_so_far, self.since))
            .or(Some(self.since));
    }

    /// Creation timestamp of the lock
    fn since(&self, semaphore: &str) -> Option<Instant> {
        if self.semaphore == semaphore {
            Some(self.since)
        } else {
            None
        }
    }
}

/// Accumulated counts for an indiviual Semaphore
#[derive(Default)]
pub struct Counts {
    /// Accumulated count of acquired leases (aka. the count) of the semaphore.
    pub acquired: i64,
    /// Accumulated count of pending leases.
    pub pending: i64,
    /// The earliest pending lock
    longest_pending_since: Option<Instant>,
}

impl Counts {
    /// Time the longest pending peer is waiting until `now`, to acquire a lock to a semaphore.
    pub fn longest_pending(&self, now: Instant) -> Duration {
        self.longest_pending_since
            .map(|earlier| now.duration_since(earlier))
            .unwrap_or_default()
    }
}

/// A peer holds leases to semaphores, which may either be active or pending and share a common
/// expiration time.
struct Peer {
    /// A peer may acquire locks to multiple semaphores.
    acquired: HashMap<String, i64>,
    /// Only one lock can be pending for any given peer.
    pending: Option<Lock>,
    /// Instant upon which the lease may be removed by litter collection.
    valid_until: Instant,
}

impl Peer {
    /// Creates a new Peer instance with no locks associated.
    fn new(valid_until: Instant) -> Self {
        Self {
            acquired: HashMap::new(),
            pending: None,
            valid_until,
        }
    }

    fn count_acquired(&self, semaphore: &str) -> i64 {
        self.acquired.get(semaphore).copied().unwrap_or(0)
    }

    /// Semaphore count of this peer regardless of wether the lock is acquired or pending.
    fn count_demand(&self, semaphore: &str) -> i64 {
        // Look in pending
        let pending = self
            .pending
            .as_ref()
            .map(|l| l.count(semaphore))
            .unwrap_or(0);
        if pending != 0 {
            pending
        } else {
            // Not found in pending, look in acquired
            self.count_acquired(semaphore)
        }
    }

    /// Increments the suitable entries in `counts`.
    fn update_counts(&self, counts: &mut HashMap<String, Counts>) {
        for (semaphore, count) in &self.acquired {
            counts
                .get_mut(semaphore)
                .expect("Only known semaphores must be managed by peers.")
                .acquired += count;
        }
        if let Some(lock) = &self.pending {
            lock.update_counts_pending(counts);
        }
    }

    /// Empties the peer. Returns name of associated semaphores, if available.
    fn clear(&mut self) -> Vec<String> {
        self.pending
            .take()
            .map(|l| l.semaphore)
            .into_iter()
            .chain(self.acquired.drain().map(|(semaphore, _count)| semaphore))
            .collect()
    }

    // Check wether the peer is expired. If so its `lock` will be released.
    //
    // # Return
    //
    // Outer `Option` indicates wether the peer is still valid. If so it is `None`. If not the inner
    // Option holds the name of the semaphores if available.
    fn remove_expired(&mut self, now: Instant) -> Option<Vec<String>> {
        if self.valid_until < now {
            // Peer is expired
            Some(self.clear())
        } else {
            // Peer is still valid
            None
        }
    }

    /// True if the locks associated with this peer are acquired
    fn all_acquired(&self) -> bool {
        self.pending.is_none()
    }

    /// Adds a lock for the semaphore to the peer
    fn add_lock(&mut self, semaphore: String, count: i64, acquired: bool) {
        if acquired {
            let prev = self.acquired.insert(semaphore, count);
            debug_assert!(prev.is_none());
        } else {
            let since = Instant::now();
            debug_assert!(self.pending.is_none());
            self.pending = Some(Lock {
                semaphore,
                count,
                since,
            });
        }
    }

    /// Instance since when the peer is waiting for the semaphore.
    fn pending_since(&self, semaphore: &str) -> Option<Instant> {
        self.pending.as_ref().and_then(|lock| lock.since(semaphore))
    }

    /// If the count of remainder is sufficient the pending lock is going to be promoted to
    /// acquired. The remainder is decremented independent of wether the lease could be acquired or
    /// not.
    ///
    /// # Return
    ///
    /// `true` if the lock has been acquired.
    fn try_resolve(&mut self, remainder: &mut i64) -> bool {
        *remainder -= self.pending.as_ref().unwrap().count;
        if *remainder >= 0 {
            let lock = self
                .pending
                .take()
                .expect("Peer without pending lock must not be resolved.");
            let prev = self.acquired.insert(lock.semaphore, lock.count);
            debug_assert!(prev.is_none());
            true
        } else {
            false
        }
    }

    /// Relase a pending or acquired lock, to a semaphore.
    ///
    /// # Return
    ///
    /// `true` if a pending lock has been released.
    fn release_lock(&mut self, semaphore: &str) -> bool {
        let prev = self.acquired.remove(semaphore);
        if prev.is_none() {
            self.pending.take().is_some()
        } else {
            false
        }
    }
}

/// Every peer has a unique PeerId associated with it for bookkeeping.
pub type PeerId = u64;

/// Does the bookeeping for all the peers, which 'lease' Semaphores by acquiring locks to them. This
/// is a purely a bookeeping struct and does not provide any synchronization mechanisms. Rather they
/// are build arount this type.
pub struct Leases {
    //  Peers holding pending or acquired leases to the semaphores
    ledger: HashMap<PeerId, Peer>,
}

impl Leases {
    pub fn new() -> Self {
        Leases {
            ledger: HashMap::new(),
        }
    }

    /// Creates a new empty peer with no locks.
    ///
    /// # Return
    ///
    /// The id identifying the new peer. Used as a key in this datastructure to access and
    /// manipulate its state.
    pub fn new_peer(&mut self, valid_until: Instant) -> PeerId {
        let id = self.new_unique_peer_id();
        let old = self.ledger.insert(id, Peer::new(valid_until));
        // There should not be any preexisting entry with this id
        debug_assert!(old.is_none());
        id
    }

    /// Creates a new peer with an existing peer id.
    ///
    /// This is useful, to restore revenants (i.e. Peers for which we receive a heartbeat after we
    /// removed them, due to expiration). This way we can restore them without having to change the
    /// peer id on the client side.
    ///
    /// It's the responsibility of the caller to ensure this method is not called with an already
    /// existing peer id.
    pub fn new_peer_at(&mut self, id: PeerId, valid_until: Instant) {
        let old = self.ledger.insert(id, Peer::new(valid_until));
        // There should not be any preexisting entry with this id
        assert!(old.is_none());
    }

    /// Acquires a lock for a peer. If the count of the semaphore is high enough, the lease is going
    /// to be acquired, otherwise it remains pending. This method does not block.
    ///
    /// # Parameters
    ///
    /// * `peer_id`: Should this be None, a new peer_id is going to be generated,
    ///              otherwise the provided one is used.
    /// * `max`: If set to `None` the new leasel are always going to be active. This is
    ///          useful to handle revenat peers with active leaves. If a value is set a
    ///          check is performed and the lease are only going to be active, if it
    ///          would not exceed the full counts of the involved semaphores.
    ///
    /// # Return
    ///
    /// Returns `true` if, and only if, all locks of the peer are acquired.
    pub fn acquire(
        &mut self,
        peer_id: PeerId,
        semaphore: &str,
        amount: u32,
        max: Option<i64>,
    ) -> Result<bool, ThrottleError> {
        let amount = amount as i64;

        // Compare with previous state of the lock. If the same amount for the same semaphore has
        // been demanded already, do nothing.
        let peer = self
            .ledger
            .get(&peer_id)
            .ok_or(ThrottleError::UnknownPeer)?;
        let previous_demand = peer.count_demand(semaphore);
        if previous_demand != 0 {
            match amount.cmp(&previous_demand) {
                // TODO: Reduce lock count and notify if not pending.
                Ordering::Less => return Err(ThrottleError::NotImplemented),
                Ordering::Equal => return Ok(peer.all_acquired()),
                Ordering::Greater => return Err(ThrottleError::Deadlock),
            }
        }

        // Since this creates a new peer, we know it has the most recent timestamp. This implies
        // that due to fairness every other peer has priority over this one taking the lock.
        // Therfore we only take the lock (if a check on max is performed at all), if the total
        // demand is smaller than max.semaphore_service
        //
        // Note: This is different than taking the lock from a semaphore for which the count is
        // smaller than max in situations there e.g. a lock with a count of 5 is pending with while
        // the remainder is 3.
        let acquired = max
            .map(|max| self.demand_smaller_or_equal(semaphore, max - amount))
            .unwrap_or(true);

        self.ledger
            .get_mut(&peer_id)
            .unwrap()
            .add_lock(semaphore.to_owned(), amount, acquired);

        Ok(acquired)
    }

    /// Aggregated count of active leases for the semaphore
    pub fn count(&self, semaphore: &str) -> i64 {
        self.ledger
            .values()
            .map(|lease| lease.count_acquired(semaphore))
            .sum()
    }

    /// Should a peer with `peer_id` be found, it is removed and the names of the semaphores it
    /// holds is returned.
    pub fn remove_peer(&mut self, peer_id: PeerId) -> Result<Vec<String>, ThrottleError> {
        let semaphores = self
            .ledger
            .remove(&peer_id)
            .ok_or(ThrottleError::UnknownPeer)?
            .clear();

        Ok(semaphores)
    }

    /// Acquires pending leases for the semaphore until its count is >= max. It acquires the locks
    /// pending the longest first.
    pub fn resolve_pending(&mut self, semaphore: &str, max: i64, resolved_peers: &mut Vec<PeerId>) {
        let mut remainder = max - self.count(semaphore);
        while let Some(peer_id) = self.resolve_highest_priority_pending(semaphore, &mut remainder) {
            resolved_peers.push(peer_id);
        }
    }

    /// Wether the peer has any pending leases.
    ///
    /// # Return
    ///
    /// `true` if peer has any pending leases, `false` if not. Returns `UnknownPeer` if peer does
    /// not exist.
    pub fn has_pending(&self, peer_id: PeerId) -> Result<bool, ThrottleError> {
        self.ledger
            .get(&peer_id)
            .ok_or(ThrottleError::UnknownPeer)
            .map(|peer| !peer.all_acquired())
    }

    /// Remove every lease, which is not valid until now.
    ///
    /// Under ordinary circumstances leases should be explicitly removed. Yet a client may die due
    /// to an error and never get a chance to free the lease. Therfore we free this litter on
    /// ocation. After calling this resolved pending should be invoked on the affected semaphores.
    ///
    /// # Return
    ///
    /// (List of expired peers, affected_semaphores)
    pub fn remove_expired(&mut self, now: Instant) -> (Vec<PeerId>, Vec<String>) {
        let mut expired_peers = Vec::new();
        let mut affected_semaphores = Vec::new();
        self.ledger.retain(|peer_id, peer| {
            if let Some(semaphores) = peer.remove_expired(now) {
                // Peer is expired
                expired_peers.push(*peer_id);
                affected_semaphores.extend(semaphores);
                // Don't retain this peer in the ledger
                false
            } else {
                // Not expired, let's keep this one
                true
            }
        });
        // Remove duplicates
        affected_semaphores.sort();
        affected_semaphores.dedup();
        (expired_peers, affected_semaphores)
    }

    /// Called to increase the timestamp of a lease to prevent it from expiring.
    ///
    /// # Return
    ///
    /// May return `ThrottleError::UnknownPeer` if `peer_id` is not found.
    pub fn update_valid_until(
        &mut self,
        peer_id: PeerId,
        valid_until: Instant,
    ) -> Result<(), ThrottleError> {
        let peer = self
            .ledger
            .get_mut(&peer_id)
            .ok_or(ThrottleError::UnknownPeer)?;
        peer.valid_until = valid_until;
        Ok(())
    }

    /// Fills counts with the current accumulated counts for each semaphore. One entry for each
    /// semaphore must already be present in the hash map. Otherwise this method panics.
    pub fn fill_counts(&self, counts: &mut HashMap<String, Counts>) {
        for lease in self.ledger.values() {
            lease.update_counts(counts);
        }
    }

    /// Release a lock associated with a peer.
    ///
    /// # Return
    ///
    /// `true` if the lock had been pending.
    pub fn release_lock(
        &mut self,
        peer_id: PeerId,
        semaphore: &str,
    ) -> Result<bool, ThrottleError> {
        let was_pending = self
            .ledger
            .get_mut(&peer_id)
            .ok_or(ThrottleError::UnknownPeer)?
            .release_lock(semaphore);
        Ok(was_pending)
    }

    /// Generates a random new peer id which does not collide with any preexisting
    fn new_unique_peer_id(&self) -> PeerId {
        loop {
            let candidate = random();
            if self.ledger.get(&candidate).is_none() {
                return candidate;
            }
        }
    }

    /// Returns wether the demand (i.e. the sum of pending and acquired lock counts) for this lease
    /// is smaller or equal to `max`.
    fn demand_smaller_or_equal(&self, semaphore: &str, max: i64) -> bool {
        let mut demand = 0;
        debug_assert!(max >= 0);
        // If we would allow lock counts > semahpore counts in this method we'd need these lines. So
        // the condition is always false.
        // if demand > max {
        //     return false;
        // }
        for peer in self.ledger.values() {
            demand += peer.count_demand(semaphore);
            // early return
            if demand > max {
                return false;
            }
        }
        true
    }

    /// Return the pending lock with the highest priority for this semaphore. Since we have fair
    /// semaphores, this is the peer waiting the longest. Returns `None` in case there are not any
    /// pending locks.
    fn resolve_highest_priority_pending<'a>(
        &'a mut self,
        semaphore: &str,
        remainder: &mut i64,
    ) -> Option<PeerId> {
        let min = self
            .ledger
            .iter_mut()
            .filter_map(|(id, peer)| peer.pending_since(semaphore).map(|since| (id, peer, since)))
            .min_by_key(|(_id, _peer, since)| *since);

        if let Some((&id, peer, _since)) = min {
            // Decrements the remainder of the amount, regardless of wether we acquire it or not
            // doing so prevents us from starving locks requesting big amounts.
            if peer.try_resolve(remainder) {
                Some(id)
            } else {
                None
            }
        } else {
            None
        }
    }
}
