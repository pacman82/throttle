//! This module exposes the state of the server via an HTTP interface. As such its primary concerns
//! are modding success and error states to HTTP status codes. Defining in which formats to
//! deserialize paramaters and serialize respones, or deciding on which HTTP methods to map the
//! functions.

use crate::{error::ThrottleError, leases::PeerId, service_interface::Api, state::AppState};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use percent_encoding::percent_decode;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub fn semaphores() -> Router<Arc<AppState>> {
    Router::new()
        .route("/peers/:id/:semaphore", put(acquire))
        .route("/peers/:id/:semaphore", delete(release_lock))
        .route("/restore", post(restore))
        .route("/remainder", get(remainder))
        .route("/peers/:id/is_acquired", get(is_acquired))
        .route("/peers/:id", put(put_peer))
}

pub fn semaphores2() -> Router<Api> {
    Router::new()
        .route("/new_peer", post(new_peer))
        .route("/peers/:id", delete(release))
}

impl ThrottleError {
    fn status_code(&self) -> StatusCode {
        match self {
            ThrottleError::UnknownPeer
            | ThrottleError::UnknownSemaphore
            | ThrottleError::InvalidLockCount { .. } => StatusCode::BAD_REQUEST,
            ThrottleError::Never { .. }
            | ThrottleError::Deadlock { .. }
            | ThrottleError::ChangeThroughRestore
            | ThrottleError::AlreadyPending => StatusCode::CONFLICT,
            ThrottleError::ShrinkingLockCount => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

impl IntoResponse for ThrottleError {
    fn into_response(self) -> Response {
        Response::builder()
            .status(self.status_code())
            .body(Body::new(self.to_string()))
            .unwrap()
    }
}

type Locks = HashMap<String, i64>;

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
async fn new_peer(mut api: State<Api>, body: Json<ExpiresIn>) -> Json<PeerId> {
    let peer_id = api.new_peer(body.expires_in).await;
    Json(peer_id)
}

async fn release(mut api: State<Api>, path: Path<PeerId>) -> Json<&'static str> {
    if api.release_peer(*path).await {
        Json("Peer released")
    } else {
        // Post condition of lease not being there is satisfied, let's make this request 200 still.
        Json("Peer not found")
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
async fn acquire(
    state: State<Arc<AppState>>,
    Path((peer_id, semaphore)): Path<(PeerId, String)>,
    query: Query<AcquireQuery>,
    body: Json<i64>,
) -> Result<(StatusCode, Json<PeerId>), ThrottleError> {
    let semaphore = percent_decode(semaphore.as_bytes()).decode_utf8_lossy();

    let amount = body.0;
    // Turn `Option<HumantimeDuratino>` into `Option<Duration>`.
    let wait_for = query.block_for.map(|hd| hd.0);
    let expires_in = query.expires_in.map(|hd| hd.0);
    if state
        .acquire(peer_id, &semaphore, amount, wait_for, expires_in)
        .await?
    {
        Ok((StatusCode::OK, Json(peer_id)))
    } else {
        Ok((StatusCode::ACCEPTED, Json(peer_id)))
    }
}

async fn release_lock(
    Path((peer_id, semaphore)): Path<(PeerId, String)>,
    state: State<Arc<AppState>>,
) -> Result<&'static str, ThrottleError> {
    let semaphore = percent_decode(semaphore.as_bytes()).decode_utf8_lossy();

    state.release_lock(peer_id, &semaphore)?;
    Ok("Ok")
}

#[derive(Deserialize)]
struct Restore {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
    peer_id: PeerId,
    acquired: Locks,
}

/// Called by the client, after receiving `Unknown Peer`. Restores the state of the peer. The
/// acquired locks of the peer are guaranteed to be acquired. Even if this means going over the full
/// count of the semaphore. Pending locks may be resolved, if this is possible without going over
/// the semaphores full count, or violating fairness.
async fn restore(
    state: State<Arc<AppState>>,
    body: Json<Restore>,
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
async fn remainder(
    state: State<Arc<AppState>>,
    query: Query<Remainder>,
) -> Result<Json<i64>, ThrottleError> {
    state.remainder(&query.semaphore).map(Json)
}

/// Returns wether all the locks of the peer have been acquired. This route will not block, but
/// return immediatly.
async fn is_acquired(
    state: State<Arc<AppState>>,
    Path(peer_id): Path<PeerId>,
) -> Result<Json<bool>, ThrottleError> {
    state.is_acquired(peer_id).map(Json)
}

async fn put_peer(
    state: State<Arc<AppState>>,
    Path(peer_id): Path<PeerId>,
    body: Json<ExpiresIn>,
) -> Result<&'static str, ThrottleError> {
    state.heartbeat(peer_id, body.expires_in)?;
    Ok("Ok")
}
