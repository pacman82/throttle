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
use actix_web::web::Data;
use axum::Router;
use clap::Parser;
use log::{info, warn};
use std::io;

use crate::cli::Cli;

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
    let state = Data::new(state::State::new(application_cfg.semaphores));

    // Copy a reference to state, before moving it into the closure. We need it later to start the
    // litter collection.
    let state_ref_lc = state.clone();

    // Without this line, the metric is only going to be initalized, after the first request to an
    // unknown resource. I.e. We would see nothing instead of `num_404 0` in the metrics route,
    // before the first request to an unknown resource.
    not_found::initialize_metrics();

    let app = Router::new()
        .route("/", axum::routing::get(index))
        .fallback(not_found::not_found);

    // let server_terminated = HttpServer::new(move || {
    //     App::new()
    //         .app_data(state.clone())
    //         .service(health::health)
    //         .service(metrics::metrics)
    //         .service(favicon::favicon)
    //         .service(version::get_version)
    //         .service(semaphore_service::new_peer)
    //         .service(semaphore_service::acquire)
    //         .service(semaphore_service::remainder)
    //         .service(semaphore_service::release)
    //         .service(semaphore_service::restore)
    //         .service(semaphore_service::remove_expired)
    //         .service(semaphore_service::put_peer)
    //         .service(semaphore_service::is_acquired)
    //         .service(semaphore_service::release_lock)
    // })
    // .bind(&opt.endpoint())?
    // .run();

    let listener = tokio::net::TcpListener::bind(&opt.endpoint()).await?;
    let server_terminated = axum::serve(listener, app);

    // Removes expired peers asynchrounously. We start litter collection after the server. Would we
    // start `lc` before the `.run` method, the ?-operator after `.bind` might early return and
    // leave us with a detached thread.
    let lc = litter_collection::start(
        state_ref_lc.into_inner(),
        application_cfg.litter_collection_interval,
    );

    let result = server_terminated.await; // Don't use ? to early return before stopping the lc.

    // Stop litter collection.
    lc.stop();

    result
}
