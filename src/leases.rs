use rand::random;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// A peer holds leases to semaphores, which may either be active or pending and share a common
/// timeout.
struct Peer {
    /// Name of the resource the semaphore protects
    semaphore: String,
    /// `true` if the lease is active (i.e. decrementing the semaphore count), or `false` if the
    /// lease is pending.
    acquired: bool,
    /// The semapohre count is decreased by `amount` if the lease is active.
    amount: i64,
    /// Instant upon which the lease may be removed by litter collection.
    valid_until: Instant,
    /// Instant of peer creation. Used to implement fairness of semaphores.
    since: Instant,
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
            .unwrap_or(Default::default())
    }
}

impl Peer {
    fn count_acquired(&self, semaphore: &str) -> i64 {
        if self.acquired && self.semaphore == semaphore {
            self.amount
        } else {
            0
        }
    }

    /// Semaphore count of this peer regardless of wether the lock is acquired or pending.
    fn count_demand(&self, semaphore: &str) -> i64 {
        if self.semaphore == semaphore {
            self.amount
        } else {
            0
        }
    }

    /// Increments the suitable entries in `counts`.
    fn update_counts(&self, counts: &mut HashMap<String, Counts>) {
        let mut counts = counts
            .get_mut(&self.semaphore)
            .expect("All available Semaphores must be prefilled in counts.");
        if self.acquired {
            counts.acquired += self.amount;
        } else {
            counts.pending += self.amount;
            // If there already has been a minimum, compare. Otherwise just use `self.since`.
            counts.longest_pending_since = counts
                .longest_pending_since
                .map(|min_so_far| std::cmp::min(min_so_far, self.since))
                .or(Some(self.since));
        }
    }
}

pub struct Leases {
    //  Peers holding pending or acquired leases to the semaphores
    ledger: HashMap<u64, Peer>,
}

impl Leases {
    pub fn new() -> Self {
        Leases {
            ledger: HashMap::new(),
        }
    }

    /// Creates a new unique peer id and adds it to the ledger. If the count of the semaphore is
    /// high enough, the lease is going to be active, otherwise it is pending.
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
    /// Returns `true` if, and only if all leases of the peer are active.
    pub fn add(
        &mut self,
        peer_id: u64,
        semaphore: &str,
        amount: u32,
        max: Option<i64>,
        valid_until: Instant,
    ) -> bool {
        let amount = amount as i64;

        // Since this creates a new peer, we know it has the most recent timestamp. This implies
        // that due to fairness every other peer has priority over this one taking the lock.
        // Therfore we only take the lock (if a check on max is performed at all), if the total
        // demand is smaller than max.semaphore_service
        //
        // Note: This is different than taking the lock from a semaphore for which the count is
        // smaller than max in situations there e.g. a lock with a count of 5 is pending with while
        // the remainder is 3.
        let acquired = max
            .map(|max| self.demand_smaller_or_equal(semaphore, max))
            .unwrap_or(true);

        let old = self.ledger.insert(
            peer_id,
            Peer {
                semaphore: semaphore.to_owned(),
                acquired,
                amount,
                valid_until,
                since: Instant::now(),
            },
        );
        // There should not be any preexisting entry with this id
        debug_assert!(old.is_none());
        acquired
    }

    /// Aggregated count of active leases for the semaphore
    pub fn count(&self, semaphore: &str) -> i64 {
        self.ledger
            .values()
            .map(|lease| lease.count_acquired(semaphore))
            .sum()
    }

    /// Should a lease with that semaphore be found, it is removed and the name of the semaphore it
    /// holds is returned.
    pub fn remove(&mut self, peer_id: u64) -> Option<String> {
        self.ledger.remove(&peer_id).map(|l| l.semaphore)
    }

    /// Acquires pending leases for the semaphore until its count is >= max. It acquires the locks
    /// pending the longest first.
    pub fn resolve_pending(&mut self, semaphore: &str, max: i64) {
        let mut remainder = max - self.count(semaphore);
        while remainder > 0 {
            if let Some(peer) = self.highest_priority_pending(semaphore) {
                // Decrement the remainder of the amount, regardless of wether we acquire it or not
                // doing so prevents us from starving locks requesting big amounts.
                remainder -= peer.amount;
                if remainder >= 0 {
                    peer.acquired = true;
                }
            } else {
                // No more pending locks to consider.
                break;
            }
        }
    }

    /// Wether the peer has any pending leases.
    pub fn has_pending(&self, peer_id: u64) -> Option<bool> {
        self.ledger.get(&peer_id).map(|lease| !lease.acquired)
    }

    /// Remove every lease, which is not valid until now.
    ///
    /// Under ordinary circumstances leases should be explicitly removed. Yet a client may die due
    /// to an error and never get a chance to free the lease. Therfore we free this litter on
    /// ocation.
    ///
    /// # Return
    ///
    /// The number of removed leases.
    pub fn remove_expired(&mut self, now: Instant) -> usize {
        let before = self.ledger.len();
        self.ledger
            .retain(|_peer_id, lease| now < lease.valid_until);
        let after = self.ledger.len();
        before - after
    }

    /// Called to increase the timestamp of a lease to prevent it from expiring.
    ///
    /// # Return
    ///
    /// Should the `peer_id` been found `true` returned. `false` otherwise.
    pub fn update_valid_until(&mut self, peer_id: u64, valid_until: Instant) -> bool {
        if let Some(lease) = self.ledger.get_mut(&peer_id) {
            lease.valid_until = valid_until;
            true
        } else {
            false
        }
    }

    /// Fills counts with the current accumulated counts for each semaphore. One entry for each
    /// semaphore must already be present in the hash map. Otherwise this method panics.
    pub fn fill_counts(&self, counts: &mut HashMap<String, Counts>) {
        for lease in self.ledger.values() {
            lease.update_counts(counts);
        }
    }

    /// Generates a random new peer id which does not collide with any preexisting
    pub fn new_unique_peer_id(&self) -> u64 {
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
        for peer in self.ledger.values() {
            demand += peer.count_demand(semaphore);
            // early return
            if demand <= max {
                return false;
            }
        }
        true
    }

    /// Return the pending lock with the highest priority for this semaphore. Since we have fair
    /// semaphores, this is the lock waiting the longest. Returs `None` in case there are not any
    /// pending locks.
    fn highest_priority_pending<'a>(&'a mut self, semaphore: &str) -> Option<&'a mut Peer> {
        self.ledger
            .values_mut()
            .filter(|peer| peer.semaphore == semaphore)
            .min_by_key(|peer| peer.since)
    }
}
