//! This module exposes the sate of the server via an HTTP interface. As such it primary concerns
//! are modding success and error states to HTTP status codes. Defining in which formats to
//! deserialize paramaters and serialize respones, or deciding on which HTTP methods to map the
//! functions.

use crate::{error::ThrottleError, leases::PeerId, state::State};
use actix_web::{
    delete, get,
    http::StatusCode,
    post, put,
    web::{Data, Json, Path, Query},
    HttpResponse, ResponseError,
};
use log::debug;
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

impl ResponseError for ThrottleError {
    fn status_code(&self) -> StatusCode {
        match self {
            ThrottleError::UnknownPeer
            | ThrottleError::UnknownSemaphore
            | ThrottleError::InvalidLockCount { .. } => StatusCode::BAD_REQUEST,
            ThrottleError::ForeverPending { .. }
            | ThrottleError::Deadlock
            | ThrottleError::ChangeThroughRestore
            | ThrottleError::AlreadyPending => StatusCode::CONFLICT,
            ThrottleError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

type Leases = HashMap<String, i64>;

/// Strict alias around `Duration`. Yet it serializes from a human readable representation.
#[derive(Deserialize, Clone, Copy)]
struct HumanDuration(#[serde(with = "humantime_serde")] Duration);

/// Used as a query parameter in requests. E.g. `?expires_in=5m`.
#[derive(Deserialize)]
struct ExpiresIn {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
}

/// Create a new peer with no acquired locks.
///
/// Returns id of the new peer
#[post("/new_peer")]
async fn new_peer(body: Json<ExpiresIn>, state: Data<State>) -> Json<PeerId> {
    Json(state.new_peer(body.expires_in))
}

#[delete("/peers/{id}")]
async fn release(path: Path<PeerId>, state: Data<State>) -> HttpResponse {
    if state.release(*path) {
        HttpResponse::Ok().json("Peer released")
    } else {
        // Post condition of lease not being there is satisfied, let's make this request 200 still.
        HttpResponse::Ok().json("Peer not found")
    }
}

/// Used as a query parameter in requests. E.g. `?expires_in=5m`.
#[derive(Deserialize)]
struct AcquireQuery {
    expires_in: Option<HumanDuration>,
    // Don't know how to use `humantime_serde` without wrapper inside an `Option`.
    block_for: Option<HumanDuration>,
}

/// Acquire a lock to a semaphore.
///
/// This function is supposed to be called repeatedly from client side, until the lock is acquired
/// It also updates the expiration timeout to prevent the litter collection from removing the peer
/// while it is pending. Having repeated short lived requests is preferable over one long running,
/// as many proxies, firewalls, and Gateways might kill them.
#[put("/peers/{id}/{semaphore}")]
async fn acquire(
    path: Path<(PeerId, String)>,
    query: Query<AcquireQuery>,
    body: Json<i64>,
    state: Data<State>,
) -> HttpResponse {
    let amount = body.0;
    let peer_id = path.0;
    let semaphore = &path.1;
    // Turn `Option<HumantimeDuratino>` into `Option<Duration>`.
    let wait_for = query.block_for.map(|hd| hd.0);
    let expires_in = query.expires_in.map(|hd| hd.0);
    match state
        .acquire(peer_id, semaphore, amount, wait_for, expires_in)
        .await
    {
        Ok(true) => HttpResponse::Ok().json(peer_id),
        Ok(false) => HttpResponse::Accepted().json(peer_id),
        Err(error) => HttpResponse::from_error(error.into()),
    }
}

#[delete("/peers/{id}/{semaphore}")]
async fn release_lock(
    path: Path<(PeerId, String)>,
    state: Data<State>,
) -> Result<&'static str, ThrottleError> {
    let peer_id = path.0;
    let semaphore = path.1.as_str();
    state.release_lock(peer_id, semaphore)?;
    Ok("Ok")
}

#[derive(Deserialize)]
pub struct Restore {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
    peer_id: PeerId,
    acquired: Leases,
}

/// Called by the client, after receiving `Unknown Peer`. Restores the state of the peer. The
/// acquired locks of the peer are guaranteed to be acquired. Even if this means going over the full
/// count of the semaphore. Pending locks may be resolved, if this is possible without going over
/// the semaphores full count, or violating fairness.
#[post("/restore")]
pub async fn restore(
    body: Json<Restore>,
    state: Data<State>,
) -> Result<&'static str, ThrottleError> {
    state.restore(body.peer_id, body.expires_in, &body.acquired)?;
    Ok("Ok")
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
async fn is_acquired(path: Path<PeerId>, state: Data<State>) -> Result<Json<bool>, ThrottleError> {
    state.is_acquired(*path).map(Json)
}

/// Manually remove all expired semapahores. Usefull for testing
#[post("/remove_expired")]
async fn remove_expired(state: Data<State>) -> Json<usize> {
    debug!("Remove expired triggered");
    Json(state.remove_expired())
}

#[put("/peers/{id}")]
async fn put_peer(
    path: Path<PeerId>,
    body: Json<ExpiresIn>,
    state: Data<State>,
) -> Result<&'static str, ThrottleError> {
    let peer_id = *path;
    state.heartbeat(peer_id, body.expires_in)?;
    Ok("Ok")
}
