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
use clap::Parser;
use log::{info, warn};
use std::io;

use crate::{cli::Cli, event_loop::EventLoop, service_interface::HttpServiceInterface};

mod application_cfg;
mod cli;
mod error;
mod event_loop;
mod favicon;
mod health;
mod leases;
mod litter_collection;
mod logging;
mod metrics;
mod not_found;
mod semaphore_service;
mod service_interface;
mod state;
mod ui;
mod version;

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

    let app = EventLoop::new(application_cfg.semaphores);

    // Removes expired peers asynchrounously. We must take care not to exit early with the
    // ?-operator in order to not be left with a detached thread.
    let lc = litter_collection::start(app.watch_valid_until(), app.api());

    let service_interface = HttpServiceInterface::new(&opt.endpoint(), app.api()).await?;

    app.run_event_loop().await;

    // Don't use ? to early return before stopping the lc.
    let result = service_interface.shutdown().await;

    // Stop litter collection.
    lc.stop().await;

    result
}
