use rand::random;
use std::{collections::HashMap, time::Instant};

/// Describes a single client lease to a semaphore
struct Lease {
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

impl Lease {
    fn count_active(&self, semaphore: &str) -> i64 {
        if self.active && self.semaphore == semaphore {
            self.amount
        } else {
            0
        }
    }

    /// Activates a pending lease if semaphore matches and remainder is positiv (>0)
    fn activate_if_viable(&mut self, semaphore: &str, remainder: &mut i64) {
        if !self.active && semaphore == self.semaphore && *remainder >= self.amount {
            self.active = true;
            *remainder -= self.amount;
        }
    }
}

pub struct Leases {
    // Active leases decreasing the semaphore count
    ledger: HashMap<u64, Lease>,
}

impl Leases {
    pub fn new() -> Self {
        Leases {
            ledger: HashMap::new(),
        }
    }

    /// Creates a new unique lease id and adds it to the ledger. If the count of the semaphore is
    /// high enough, the lease is going to be active, otherwise it is pending.Leases
    ///
    /// # Return
    /// First element indicates wether lease is active, or not.
    /// Second element is the lease id.
    pub fn add(
        &mut self,
        resource: &str,
        amount: u32,
        max: i64,
        valid_until: Instant,
    ) -> (bool, u64) {
        let amount = amount as i64;

        // Generate random numbers until we get a new unique one.
        let lease_id = loop {
            let candidate = random();
            if self.ledger.get(&candidate).is_none() {
                break candidate;
            }
        };

        let active = self.count(resource) + amount <= max;

        let old = self.ledger.insert(
            lease_id,
            Lease {
                semaphore: resource.to_owned(),
                active,
                amount,
                valid_until,
            },
        );
        // There should not be any preexisting entry with this id
        debug_assert!(old.is_none());
        (active, lease_id)
    }

    /// Aggregated count of active leases for the semaphore
    pub fn count(&self, resource: &str) -> i64 {
        self.ledger
            .values()
            .map(|lease| lease.count_active(resource))
            .sum()
    }

    /// Should a lease with that semaphore be found, it is removed and the name of the semaphore it
    /// holds is returned.
    pub fn remove(&mut self, lease_id: u64) -> Option<String> {
        self.ledger.remove(&lease_id).map(|l| l.semaphore)
    }

    /// Activates pending leases for the semaphore until its count is >= max
    pub fn resolve_pending(&mut self, semaphore: &str, max: i64) {
        let mut remainder = max - self.count(semaphore);
        for lease in self.ledger.values_mut() {
            // Return early if count is already to high
            if remainder <= 0 {
                break;
            }
            lease.activate_if_viable(semaphore, &mut remainder);
        }
    }

    pub fn is_active(&self, lease_id: u64) -> Option<bool> {
        self.ledger.get(&lease_id).map(|lease| lease.active)
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
            .retain(|_lease_id, lease| now < lease.valid_until);
        let after = self.ledger.len();
        before - after
    }

    /// Updates the timestamp of an existing lease. Does not perform a consistency check with a
    /// preexisting lease, but may insert a revenant (i.e. a previously forgotten lease) back into
    /// bookeeping.
    pub fn update(&mut self, lease_id: u64, semaphore: &str, amount: u32, valid_until: Instant) {
        self.ledger
            .entry(lease_id)
            .and_modify(|lease| lease.valid_until = valid_until)
            .or_insert(Lease {
                active: true,
                semaphore: semaphore.to_owned(),
                amount: amount as i64,
                valid_until,
            });
    }
}
