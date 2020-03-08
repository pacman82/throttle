use rand::random;
use std::{collections::HashMap, time::Instant};

/// A peer holds leases to semaphores, which may either be active or pending and share a common
/// timeout.
struct Peer {
    /// Name of the resource the semaphore protects
    semaphore: String,
    /// `true` if the lease is active (i.e. decrementing the semaphore count), or `false` if the
    /// lease is pending.
    active: bool,
    /// The semapohre count is decreased by `amount` if the lease is active.
    amount: i64,
    /// Instant upon which the lease may be removed by litter collection.
    valid_until: Instant,
}

/// Accumulated counts for an indiviual Semaphore
#[derive(Default)]
pub struct Counts {
    /// Accumulated count of active leases (aka. the count) of the semaphore.
    pub active: i64,
    /// Accumulated count of pending leases.
    pub pending: i64,
}

impl Peer {
    fn count_active(&self, semaphore: &str) -> i64 {
        if self.active && self.semaphore == semaphore {
            self.amount
        } else {
            0
        }
    }

    /// Activates a pending lease if semaphore matches and remainder is positiv (>0)
    fn activate_viable(&mut self, semaphore: &str, remainder: &mut i64) {
        if !self.active && semaphore == self.semaphore && *remainder >= self.amount {
            self.active = true;
            *remainder -= self.amount;
        }
    }

    /// Increments the suitable entries in `counts`.
    fn update_counts(&self, counts: &mut HashMap<String, Counts>) {
        let mut counts = counts
            .get_mut(&self.semaphore)
            .expect("All available Semaphores must be prefilled in counts.");
        if self.active {
            counts.active += self.amount;
        } else {
            counts.pending += self.amount;
        }
    }
}

pub struct Leases {
    // Active leases decreasing the semaphore count
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

        let active = max
            .map(|max| self.count(semaphore) + amount <= max)
            .unwrap_or(true);

        let old = self.ledger.insert(
            peer_id,
            Peer {
                semaphore: semaphore.to_owned(),
                active,
                amount,
                valid_until,
            },
        );
        // There should not be any preexisting entry with this id
        debug_assert!(old.is_none());
        active
    }

    /// Aggregated count of active leases for the semaphore
    pub fn count(&self, semaphore: &str) -> i64 {
        self.ledger
            .values()
            .map(|lease| lease.count_active(semaphore))
            .sum()
    }

    /// Should a lease with that semaphore be found, it is removed and the name of the semaphore it
    /// holds is returned.
    pub fn remove(&mut self, peer_id: u64) -> Option<String> {
        self.ledger.remove(&peer_id).map(|l| l.semaphore)
    }

    /// Activates pending leases for the semaphore until its count is >= max
    pub fn resolve_pending(&mut self, semaphore: &str, max: i64) {
        let mut remainder = max - self.count(semaphore);
        for lease in self.ledger.values_mut() {
            // Return early if count is already to high
            if remainder <= 0 {
                break;
            }
            lease.activate_viable(semaphore, &mut remainder);
        }
    }

    pub fn has_pending(&self, peer_id: u64) -> Option<bool> {
        self.ledger.get(&peer_id).map(|lease| lease.active)
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
}
