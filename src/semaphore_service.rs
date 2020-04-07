//! This module exposes the sate of the server via an HTTP interface. As such it primary concerns
//! are modding success and error states to HTTP status codes. Defining in which formats to
//! deserialize paramaters and serialize respones, or deciding on which HTTP methods to map the
//! functions.

use crate::{error::ThrottleError, state::State};
use actix_web::{
    delete, get,
    http::StatusCode,
    post, put,
    web::{Data, Json, Path, Query},
    HttpResponse, ResponseError,
};
use log::{debug, warn};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

impl ResponseError for ThrottleError {
    fn status_code(&self) -> StatusCode {
        match self {
            ThrottleError::UnknownPeer => StatusCode::BAD_REQUEST,
            ThrottleError::UnknownSemaphore => StatusCode::BAD_REQUEST,
            ThrottleError::ForeverPending { .. } => StatusCode::CONFLICT,
        }
    }
}

type Leases = HashMap<String, u32>;

/// Strict alias around `Duration`. Yet it serializes from a human readable representation.
#[derive(Deserialize)]
struct HumanDuration(#[serde(with = "humantime_serde")] Duration);

/// Used as a query parameter in requests. E.g. `?expires_in=5m`.
#[derive(Deserialize)]
struct ExpiresIn {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
}

/// Parameters for acquiring a lease
#[derive(Deserialize)]
struct PendingLeases {
    pending: Leases,
    /// Duration in seconds. After the specified time has passed the lease may be freed by litter
    /// collection.
    expires_in: HumanDuration,
}

impl PendingLeases {
    fn pending(&self) -> Option<(&str, u32)> {
        self.pending
            .iter()
            .next()
            .map(|(sem, &amount)| (sem.as_str(), amount))
    }
}

/// Parameters for heartbeat to a lease
#[derive(Deserialize)]
pub struct ActiveLeases {
    active: Leases,
    /// Duration in seconds. After the specified time has passed the lease may be freed by litter
    /// collection.
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
}

impl ActiveLeases {
    fn active(&self) -> Option<(&str, u32)> {
        self.active
            .iter()
            .next()
            .map(|(sem, &amount)| (sem.as_str(), amount))
    }
}

/// Acquire a new lease to a Semaphore
///
/// Returns id of the new peer
#[post("/new_peer")]
async fn new_peer(body: Json<ExpiresIn>, state: Data<State>) -> Json<u64> {
    Json(state.new_peer(body.expires_in))
}

/// Acquire a lock to a Semaphore. Does not block.
#[post("/peer/{id}/{semaphore}/acquire")]
async fn acquire(
    path: Path<(u64, String)>,
    query: Query<ExpiresIn>,
    body: Json<u32>,
    state: Data<State>,
) -> HttpResponse {
    let amount = body.0;
    let peer_id = path.0;
    let semaphore = &path.1;
    match state.acquire(peer_id, semaphore, amount, query.expires_in) {
        Ok((lease_id, true)) => HttpResponse::Created().json(lease_id),
        Ok((lease_id, false)) => HttpResponse::Accepted().json(lease_id),
        Err(error) => HttpResponse::from_error(error.into()),
    }
}

#[derive(Deserialize)]
struct MaxTimeout {
    timeout_ms: Option<u64>,
}

/// Waits for a ticket to be promoted to a lease
///
/// This function is supposed to be called repeatedly from client side, until the leases are
/// active. It also updates the expiration timeout to prevent the litter collection from
/// removing the peer while it is pending. Having repeated short lived requests is preferable
/// over one long running, as many proxies, firewalls, and Gateways might kill them.
///
/// ## Return
///
/// Returns `true` if leases are active.
#[post("/peers/{id}/block_until_acquired")]
async fn block_until_acquired(
    path: Path<u64>,
    query: Query<MaxTimeout>,
    body: Json<PendingLeases>,
    state: Data<State>,
) -> Result<Json<bool>, ThrottleError> {
    let lease_id = *path;
    let unblock_after = Duration::from_millis(query.timeout_ms.unwrap_or(0));
    debug!(
        "Lease {} is waiting for admission with timeout {:?}",
        lease_id, unblock_after
    );
    let peer_id = *path;
    let expires_in = body.expires_in.0;
    let acquired_in_time = if let Some((semaphore, amount)) = body.pending() {
        state
            .block_until_acquired(peer_id, expires_in, semaphore, amount, unblock_after)
            .await?
    } else {
        // Todo: How to best handle a wait for without pending leases? This path won't be triggered
        // by a correct client. Remark: Wo would not have this problem, if we handled Unknown Peer
        // at the client side.
        true
    };
    Ok(Json(acquired_in_time))
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
    state: Data<State>,
) -> Result<Json<i64>, ThrottleError> {
    state.remainder(&query.semaphore).map(Json)
}

/// Returns wether all the locks of the peer have been acquired. This route will not block, but
/// return immediatly.
#[get("/peers/{id}/is_acquired")]
async fn is_acquired(path: Path<u64>, state: Data<State>) -> Result<Json<bool>, ThrottleError> {
    state.is_acquired(*path).map(Json)
}

#[delete("/peers/{id}")]
async fn release(path: Path<u64>, state: Data<State>) -> HttpResponse {
    if state.release(*path) {
        HttpResponse::Ok().json("Peer released")
    } else {
        // Post condition of lease not being there is satisfied, let's make this request 200 still.
        HttpResponse::Ok().json("Peer not found")
    }
}

/// Manually remove all expired semapahores. Usefull for testing
#[post("/remove_expired")]
async fn remove_expired(state: Data<State>) -> Json<usize> {
    debug!("Remove expired triggered");
    Json(state.remove_expired())
}

#[put("/peers/{id}")]
async fn put_peer(
    path: Path<u64>,
    body: Json<ActiveLeases>,
    state: Data<State>,
) -> Result<&'static str, ThrottleError> {
    let lease_id = *path;
    if let Some((semaphore, amount)) = body.active() {
        debug!("Received heartbeat for {}", lease_id);
        state.heartbeat_for_active_peer(lease_id, semaphore, amount, body.expires_in)?;
    } else {
        warn!("Empty heartbeat (no active leases) for {}", lease_id);
    }
    Ok("Ok")
}

/// Query parameters for freeze
#[derive(Deserialize)]
struct FreezeQuery {
    #[serde(rename = "for")]
    time: HumanDuration,
}

#[post("/freeze")]
async fn freeze(query: Query<FreezeQuery>, state: Data<State>) -> &'static str {
    state.freeze(query.time.0);
    "Ok"
}
