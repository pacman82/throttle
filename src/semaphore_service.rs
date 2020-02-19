use crate::{application_cfg::ApplicationCfg, leases::Leases};
use actix_web::{
    delete, get,
    http::StatusCode,
    post, put,
    web::{Data, Json, Path, Query},
    HttpResponse, ResponseError,
};
use log::{debug, warn};
use serde::Deserialize;
use std::{
    fmt,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug)]
enum Error {
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

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self {
            Error::UnknownLease => StatusCode::BAD_REQUEST,
            Error::UnknownSemaphore => StatusCode::BAD_REQUEST,
        }
    }
}

/// Variadic state of the Semaphore service
pub struct State {
    /// Bookeeping for leases, protected by mutex
    leases: Mutex<Leases>,
    /// Condition variable. Notify is called thenever a lease is released, so it's suitable for
    /// blocking on request to pending leases.
    released: Condvar,
}

impl State {
    /// Creates the state required for the semaphore service
    pub fn new() -> State {
        State {
            leases: Mutex::new(Leases::new()),
            released: Condvar::new(),
        }
    }

    /// Removes leases outdated due to timestamp. Wakes threads waiting for pending leases if any
    /// leases are removed.
    ///
    /// Returns number of (now removed) expired leases
    fn remove_expired(&self) -> usize {
        let mut leases = self.leases.lock().unwrap();
        let num_removed = leases.remove_expired(Instant::now());
        if num_removed != 0 {
            self.released.notify_all();
        }
        num_removed
    }

    fn is_active(&self, lease_id: u64, timeout: Duration) -> Result<bool, Error> {
        let mut leases = self.leases.lock().unwrap();
        let start = Instant::now();
        loop {
            break match leases.is_active(lease_id) {
                None => {
                    warn!("Unknown lease accessed in is_pendig request.");
                    Err(Error::UnknownLease)
                }
                Some(true) => Ok(false),
                Some(false) => {
                    let elapsed = start.elapsed(); // Uses a monotonic system clock
                    if elapsed >= timeout {
                        // Lease is pending, even after timeout is passed
                        Ok(true)
                    } else {
                        // Lease is pending, but timeout hasn't passed yet. Let's wait for changes.
                        let (mutex_guard, wait_time_result) = self
                            .released
                            .wait_timeout(leases, timeout - elapsed)
                            .unwrap();
                        if wait_time_result.timed_out() {
                            Ok(true)
                        } else {
                            leases = mutex_guard;
                            continue;
                        }
                    }
                }
            };
        }
    }

    fn update(&self, lease_id: u64, semaphore: &str, amount: u32, valid_for: Duration) {
        let mut leases = self.leases.lock().unwrap();
        let valid_until = Instant::now() + valid_for;
        leases.update(lease_id, semaphore, amount, valid_until);
    }
}

/// Query parameters for acquiring a lease to a semaphore
#[derive(Deserialize)]
struct LeaseDescription {
    /// The name of the semaphore to be acquired
    semaphore: String,
    /// The amount by which to decrease the semaphore count if the lease is active.
    amount: u32,
    /// Duration in seconds. After the specified time has passed the lease may be freed by litter
    /// collection.
    valid_for_sec: u64,
}

/// Acquire a new lease to a Semaphore
#[post("/acquire")]
async fn acquire(
    body: Json<LeaseDescription>,
    cfg: Data<ApplicationCfg>,
    state: Data<State>,
) -> HttpResponse {
    if let Some(&max) = cfg.semaphores.get(&body.semaphore) {
        let mut leases = state.leases.lock().unwrap();
        let valid_until = Instant::now() + Duration::from_secs(body.valid_for_sec);
        let (active, lease_id) = leases.add(&body.semaphore, body.amount, max, valid_until);
        if active {
            debug!("Lease {} to '{}' acquired.", lease_id, body.semaphore);
            HttpResponse::Created().json(lease_id)
        } else {
            debug!("Ticket {} for '{}' created.", lease_id, body.semaphore);
            HttpResponse::Accepted().json(lease_id)
        }
    } else {
        warn!("Unknown semaphore '{}' requested", body.semaphore);
        HttpResponse::from_error(Error::UnknownSemaphore.into())
    }
}

#[derive(Deserialize)]
struct IsPending {
    timeout_ms: Option<u64>,
}

/// Wait for a ticket to be promoted to a lease
#[get("/leases/{id}/is_pending")]
async fn is_pending(
    path: Path<u64>,
    query: Query<IsPending>,
    state: Data<State>,
) -> Result<Json<bool>, Error> {
    let lease_id = *path;
    let timeout = Duration::from_millis(query.timeout_ms.unwrap_or(0));
    state.is_active(lease_id, timeout).map(Json)
}

/// Query parameters for getting remaining semaphore count
#[derive(Deserialize)]
struct Remainder {
    semaphore: String,
}

/// Get the remainder of a semaphore
#[get("/remainder")]
async fn remainder(
    query: Query<Remainder>,
    cfg: Data<ApplicationCfg>,
    state: Data<State>,
) -> Result<Json<i64>, Error> {
    if let Some(full_count) = cfg.semaphores.get(&query.semaphore) {
        let leases = state.leases.lock().unwrap();
        let count = leases.count(&query.semaphore);
        Ok(Json(full_count - count))
    } else {
        warn!("Unknown semaphore requested");
        Err(Error::UnknownSemaphore)
    }
}

#[delete("/leases/{id}")]
async fn release(path: Path<u64>, cfg: Data<ApplicationCfg>, state: Data<State>) -> &'static str {
    let mut leases = state.leases.lock().unwrap();
    match leases.remove(*path) {
        Some(semaphore) => {
            let full_count = cfg
                .semaphores
                .get(&semaphore)
                .expect("An active semaphore must always be configured");
            leases.resolve_pending(&semaphore, *full_count);
            // Notify waiting requests that lease has changed
            state.released.notify_all();
        }
        None => warn!("Deletion of unknown lease."),
    }
    "Ok"
}

/// Manually remove all expired semapahores. Usefull for testing
#[post("/remove_expired")]
async fn remove_expired(state: Data<State>) -> Json<usize> {
    Json(state.remove_expired())
}

#[put("/leases/{id}")]
async fn put_lease(
    path: Path<u64>,
    body: Json<LeaseDescription>,
    state: Data<State>,
) -> Result<&'static str, Error> {
    let lease_id = *path;
    let valid_for = Duration::from_secs(body.valid_for_sec);
    state.update(lease_id, &body.semaphore, body.amount, valid_for);
    Ok("Ok")
}
