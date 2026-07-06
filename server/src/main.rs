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
use app::App;
use clap::Parser;
use configuration::Configuration;
use log::info;

use crate::{cli::Cli, shutdown::shutdown_signal};

mod app;
mod cli;
mod configuration;
mod error;
mod event_loop;
mod favicon;
mod health;
mod http_shell;
mod leases;
mod logging;
mod metrics;
mod not_found;
mod semaphore_shell;
mod shutdown;
mod state;
mod version;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Register shutdown signal handlers early, so Ctrl+C / SIGTERM are caught even while we are
    // still starting up.
    let shutdown = shutdown_signal().await;

    let opt = Cli::parse();

    let cfg = Configuration::init(&opt.configuration)?;

    logging::init(&cfg.logging);

    info!(target: "app", "Starting");
    let app = App::new(cfg, opt.endpoint()).await?;
    info!(target: "app", "Ready");

    // Run until a shutdown signal is received.
    shutdown.await;

    info!(target: "app", "Shutdown signal received");
    app.shutdown().await?;
    info!(target: "app", "Shutdown complete");
    Ok(())
}
