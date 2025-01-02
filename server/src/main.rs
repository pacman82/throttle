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
use application_cfg::ApplicationCfg;
use clap::Parser;
use litter_collection::LitterCollection;
use log::{info, warn};
use tokio::net::ToSocketAddrs;
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
mod version;

struct App {
    event_loop: EventLoop,
    litter_collection: LitterCollection,
    service_interface: HttpServiceInterface,
}

impl App {
    /// Constructs the application including http interface. Application will accept request, once
    /// the future is completed, but it will only actually answer them once `run` is executed.
    pub async fn new(application_cfg: ApplicationCfg, endpoint: impl ToSocketAddrs) -> io::Result<Self> {
        if application_cfg.semaphores.is_empty() {
            warn!("No semaphores configured.")
        }
        let event_loop = EventLoop::new(application_cfg.semaphores);
        let service_interface = HttpServiceInterface::new(endpoint, event_loop.api()).await?;

        // Removes expired peers asynchrounously. We must take care not to exit early with the
        // ?-operator in order to not be left with a detached thread.
        let litter_collection = litter_collection::start(event_loop.watch_valid_until(), event_loop.api());


        let app = App { event_loop, litter_collection, service_interface };
        Ok(app)
    }

    pub async fn run(self) -> io::Result<()> {
        self.event_loop.run().await;

        // Don't use ? to early return before stopping the lc.
        let result = self.service_interface.shutdown().await;

        // Stop litter collection.
        self.litter_collection.stop().await;

        result
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let opt = Cli::parse();

    let application_cfg = match ApplicationCfg::init(&opt.configuration) {
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

    let app = App::new(application_cfg, opt.endpoint()).await?;

    app.run().await?;
    Ok(())
}
