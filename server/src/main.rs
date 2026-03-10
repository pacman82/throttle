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
use std::io;

use crate::cli::Cli;

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
mod state;
mod version;

#[tokio::main]
async fn main() -> io::Result<()> {
    let opt = Cli::parse();

    let Ok(application_cfg) = Configuration::init(&opt.configuration) else {
        return Ok(());
    };

    logging::init(&application_cfg.logging);

    info!("Hello From Throttle");

    let app = App::new(application_cfg, opt.endpoint()).await?;

    app.run().await?;
    Ok(())
}
