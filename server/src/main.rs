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
use log::{info, warn};
use service_interface::{Api, ServiceEvent};
use state::AppState;
use std::io;
use tokio::{spawn, sync::mpsc};

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

    let app = Application::new(application_cfg);

    // Removes expired peers asynchrounously. We must take care not to exit early with the
    // ?-operator in order to not be left with a detached thread.
    let lc = litter_collection::start(app.app_state(), app.api());

    let service_interface = HttpServiceInterface::new(&opt.endpoint(), app.api()).await?;

    app.run_event_loop().await;

    // Don't use ? to early return before stopping the lc.
    let result = service_interface.shutdown().await;

    // Stop litter collection.
    lc.stop().await;

    result
}

struct Application {
    event_receiver: mpsc::Receiver<ServiceEvent>,
    api: Api,
    app_state: AppState,
}

impl Application {
    pub fn new(config: ApplicationCfg) -> Self {
        let app_state = AppState::new(config.semaphores);
        let (sender, event_receiver) = mpsc::channel(5);
        let api = Api::new(sender);
        Application {
            event_receiver,
            api,
            app_state,
        }
    }

    /// Provides an Api to interfaces and actors, in order to send events to the application logic.
    pub fn api(&self) -> Api {
        self.api.clone()
    }

    pub fn app_state(&self) -> &AppState {
        &self.app_state
    }

    pub async fn run_event_loop(mut self) {
        while let Some(event) = self.event_receiver.recv().await {
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
                    let acquired_future = self
                        .app_state
                        .acquire(peer_id, semaphore, amount, wait_for, expires_in);
                    spawn(async move {
                        let acquired = acquired_future.await;
                        answer_acquired.send(acquired).unwrap()
                    });
                }
                ServiceEvent::ReleaseLock {
                    peer_id,
                    semaphore,
                    answer_release,
                } => {
                    let result = self.app_state.release_lock(peer_id, &semaphore);
                    answer_release.send(result).unwrap();
                }
                ServiceEvent::IsAcquired {
                    peer_id,
                    answer_is_aquired,
                } => {
                    let result = self.app_state.is_acquired(peer_id);
                    answer_is_aquired.send(result).unwrap();
                }
                ServiceEvent::Heartbeat {
                    peer_id,
                    expires_in,
                    answer_heartbeat,
                } => {
                    let result = self.app_state.heartbeat(peer_id, expires_in);
                    answer_heartbeat.send(result).unwrap();
                }
                ServiceEvent::Remainder {
                    semaphore,
                    answer_remainder,
                } => {
                    let result = self.app_state.remainder(&semaphore);
                    answer_remainder.send(result).unwrap()
                }
                ServiceEvent::Restore {
                    peer_id,
                    expires_in,
                    acquired,
                    answer_restore,
                } => {
                    let result = self.app_state.restore(peer_id, expires_in, &acquired);
                    answer_restore.send(result).unwrap();
                }
                ServiceEvent::UpdateMetrics {
                    answer_update_metrics,
                } => {
                    self.app_state.update_metrics();
                    answer_update_metrics.send(()).unwrap()
                }
                ServiceEvent::RemovedExpired {
                    answer_remove_expired,
                } => {
                    let num_expired = self.app_state.remove_expired();
                    answer_remove_expired.send(num_expired).unwrap();
                }
            }
        }
    }
}
