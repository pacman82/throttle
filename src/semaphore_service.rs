//! This module exposes the sate of the server via an HTTP interface. As such it primary concerns
//! are modding success and error states to HTTP status codes. Defining in which formats to
//! deserialize paramaters and serialize respones, or deciding on which HTTP methods to map the
//! functions.

use crate::state::{Error, State};
use actix_web::{
    delete, get,
    http::StatusCode,
    post, put,
    web::{Data, Json, Path, Query},
    HttpResponse, ResponseError,
};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self {
            Error::UnknownLease => StatusCode::BAD_REQUEST,
            Error::UnknownSemaphore => StatusCode::BAD_REQUEST,
        }
    }
}

type Admissions = HashMap<String, u32>;

/// Query parameters for acquiring a lease to a semaphore
#[derive(Deserialize)]
pub struct LeaseDescription {
    active: Admissions,
    pending: Admissions,
    /// Duration in seconds. After the specified time has passed the lease may be freed by litter
    /// collection.
    valid_for_sec: u64,
}

/// Acquire a new lease to a Semaphore
#[post("/acquire")]
async fn acquire(body: Json<LeaseDescription>, state: Data<State>) -> HttpResponse {
    let mut it = body.pending.iter();
    if let Some((semaphore, &amount)) = it.next() {
        match state.acquire(semaphore, amount, Duration::from_secs(body.valid_for_sec)) {
            Ok((lease_id, true)) => HttpResponse::Created().json(lease_id),
            Ok((lease_id, false)) => HttpResponse::Accepted().json(lease_id),
            Err(error) => HttpResponse::from_error(error.into()),
        }
    } else {
        HttpResponse::BadRequest().json("Empty leases are not supported, yet.")
    }
}

#[derive(Deserialize)]
struct IsPending {
    timeout_ms: Option<u64>,
}

/// Wait for a ticket to be promoted to a lease
#[post("/leases/{id}/wait_on_pending")]
async fn wait_on_pending(
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
async fn remainder(query: Query<Remainder>, state: Data<State>) -> Result<Json<i64>, Error> {
    state.remainder(&query.semaphore).map(Json)
}

#[delete("/leases/{id}")]
async fn release(path: Path<u64>, state: Data<State>) -> HttpResponse {
    if state.release(*path) {
        HttpResponse::Ok().json("Lease released")
    } else {
        // Post condition of lease not being there is satisfied, let's make this request 200 still.
        HttpResponse::Ok().json("Lease not found")
    }
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
    let mut it = body.active.iter();
    if let Some((semaphore, &amount)) = it.next() {
        state.update(
            lease_id,
            semaphore,
            amount,
            Duration::from_secs(body.valid_for_sec),
        );
    }
    // else {
    //     HttpResponse::BadRequest().json("Empty leases are not supported, yet.")
    // }

    Ok("Ok")
}
