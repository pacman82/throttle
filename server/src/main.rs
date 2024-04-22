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
use service_interface::{ServiceEvent, ServiceInterface};
use state::AppState;
use std::{io, sync::Arc};

use crate::{cli::Cli, service_interface::HttpServiceInterface};

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

    let service_interface =
        HttpServiceInterface::new(application_cfg.semaphores, &opt.endpoint()).await?;
    let app = Application::new(service_interface);
    app.run().await
}

struct Application<I> {
    service_interface: I,
    app_state: Arc<AppState>,
}

impl<I> Application<I> {
    pub fn new(service_interface: I) -> Application<I>
    where
        I: ServiceInterface,
    {
        let app_state = service_interface.app_state();
        Application {
            app_state,
            service_interface,
        }
    }

    pub async fn run(mut self) -> io::Result<()>
    where
        I: ServiceInterface,
    {
        // Removes expired peers asynchrounously. We must take care not to exit early with the
        // ?-operator in order to not be left with a detached thread.
        let lc = litter_collection::start(self.app_state.clone());

        while let Some(event) = self.service_interface.event().await {
            match event {
                ServiceEvent::NewPeer {
                    answer_peer_id,
                    expires_in,
                } => {
                    let peer_id = self.app_state.new_peer(expires_in);
                    answer_peer_id.send(peer_id).unwrap();
                }
                ServiceEvent::ReleasePeer {
                    answer_removed,
                    peer_id,
                } => {
                    let removed = self.app_state.release(peer_id);
                    answer_removed.send(removed).unwrap();
                }
                ServiceEvent::AcquireLock {
                    answer_acquired,
                    peer_id,
                    semaphore,
                    amount,
                    wait_for,
                    expires_in,
                } => {
                    let acquired = self
                        .app_state
                        .acquire(peer_id, &semaphore, amount, wait_for, expires_in)
                        .await;
                    answer_acquired.send(acquired).unwrap()
                }
            }
        }

        // Don't use ? to early return before stopping the lc.
        let result = self.service_interface.shutdown().await;

        // Stop litter collection.
        lc.stop().await;

        result
    }
}
