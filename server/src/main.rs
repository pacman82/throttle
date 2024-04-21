//! # Provide semaphores for distributed systems via an http interface.
//!
//! ## Endpoints
//!
//! * `/`: Prints a plain text greeting message, so users now what kind of server is running.
//! * `/health`: Always returns 200 ok
//! * `/metrics`: Endpoint for prometheus metrics
//! * `/favicon`: Returns throttle Icon
//!
//! Http interface for acquiring and releasing semaphores is not stable yet.
#[macro_use]
extern crate prometheus;
use axum::{routing::get, Router};
use clap::Parser;
use log::{info, warn};
use std::{io, sync::Arc};
use tokio::signal::ctrl_c;

use crate::{cli::Cli, semaphore_service::semaphores, state::AppState};

mod application_cfg;
mod cli;
mod error;
mod favicon;
mod health;
mod leases;
mod litter_collection;
mod logging;
mod metrics;
mod not_found;
mod semaphore_service;
mod state;
mod version;

async fn index() -> &'static str {
    "Hello from Throttle!"
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let opt = Cli::parse();

    let application_cfg = match application_cfg::ApplicationCfg::init(&opt.configuration) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "Couldn't parse {}:\n{}",
                opt.configuration.to_string_lossy(),
                e
            );
            return Ok(());
        }
    };

    logging::init(&application_cfg.logging);

    info!("Hello From Throttle");

    if application_cfg.semaphores.is_empty() {
        warn!("No semaphores configured.")
    }

    // We only want to use one Map of semaphores across all worker threads. To do this we wrap it in
    // `Data` which uses an `Arc` to share it between threads.
    let state = Arc::new(AppState::new(application_cfg.semaphores));

    // Copy a reference to state, before moving it into the closure. We need it later to start the
    // litter collection.
    let state_ref_lc = state.clone();

    // Without this line, the metric is only going to be initalized, after the first request to an
    // unknown resource. I.e. We would see nothing instead of `num_404 0` in the metrics route,
    // before the first request to an unknown resource.
    not_found::initialize_metrics();

    let app: Router = Router::new()
        .route("/metrics", get(metrics::metrics))
        .merge(semaphores())
        .with_state(state.clone())
        // Stateless routes
        .route("/", get(index))
        .route("/health", get(health::health))
        .route("/favicon.ico", get(favicon::favicon))
        .route("/version", get(version::version))
        .fallback(not_found::not_found);

    let listener = tokio::net::TcpListener::bind(&opt.endpoint()).await?;
    let server_terminated =
        axum::serve(listener, app).with_graceful_shutdown(async { ctrl_c().await.unwrap() });

    // Removes expired peers asynchrounously. We start litter collection after the server. Would we
    // start `lc` before the `.run` method, the ?-operator after `.bind` might early return and
    // leave us with a detached thread.
    let lc = litter_collection::start(state_ref_lc);

    let result = server_terminated.await; // Don't use ? to early return before stopping the lc.

    // Stop litter collection.
    lc.stop().await;

    result
}
