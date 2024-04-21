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
use application_cfg::SemaphoreCfg;
use clap::Parser;
use log::{info, warn};
use service_interface::ServiceInterface;
use std::{collections::HashMap, io, sync::Arc};

use crate::{cli::Cli, service_interface::HttpServiceInterface, state::AppState};

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
mod service_interface;
mod state;
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

    let service_interface = HttpServiceInterface::new(opt.endpoint());
    let app = Application::new(application_cfg.semaphores, service_interface);
    app.run().await
}

struct Application<I> {
    // We only want to use one Map of semaphores across all worker threads. To do this we wrap it in
    // an `Arc` to share it between threads.
    state: Arc<AppState>,
    service_interface: I,
}

impl<I> Application<I> {
    pub fn new(semaphores_cfg: HashMap<String, SemaphoreCfg>, service_interface: I) -> Application<I> {
        let state = Arc::new(AppState::new(semaphores_cfg));
        Application { state, service_interface }
    }

    pub async fn run(mut self) -> io::Result<()> where I: ServiceInterface {
        // Removes expired peers asynchrounously. We must take care not to exit early with the
        // ?-operator in order to not be left with a detached thread.
        let lc = litter_collection::start(self.state.clone());

        let server_terminated = self.service_interface.run_service(self.state);

        let result = server_terminated.await; // Don't use ? to early return before stopping the lc.

        // Stop litter collection.
        lc.stop().await;

        result
    }
}
