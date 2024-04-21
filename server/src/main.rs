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
use axum::{routing::get, Router};
use clap::Parser;
use log::{info, warn};
use std::{collections::HashMap, future::Future, io, sync::Arc};

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

    let service_interface = HttpServiceInterface::new(opt.endpoint());
    let app = Application::new(application_cfg.semaphores, service_interface);
    app.run().await
}

pub trait ServiceInterface {
    fn run_service(&mut self, app_state: Arc<AppState>) -> impl Future<Output=io::Result<()>>;
}

pub struct HttpServiceInterface {
    endpoint: String,
}

impl HttpServiceInterface {
    pub fn new(endpoint: String) -> Self {
        HttpServiceInterface { endpoint }
    }
}

impl ServiceInterface for HttpServiceInterface {
    async fn run_service(&mut self, app_state: Arc<AppState>) -> io::Result<()> {
        let app: Router = Router::new()
            .route("/metrics", get(metrics::metrics))
            .merge(semaphores())
            .with_state(app_state.clone())
            // Stateless routes
            .route("/", get(index))
            .route("/health", get(health::health))
            .route("/favicon.ico", get(favicon::favicon))
            .route("/version", get(version::version))
            .fallback(not_found::not_found);

        let listener = tokio::net::TcpListener::bind(&self.endpoint).await?;
        axum::serve(listener, app).await
    }
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
