//! A template for webservices written with Actix intended to be executed within stratosphere.
#[macro_use]
extern crate prometheus;
use actix_web::{get, web, web::Data, App, HttpServer};
use log::info;
use std::io;
use structopt::StructOpt;

use crate::cli::Cli;

mod application_cfg;
mod cli;
mod favicon;
mod health;
mod leases;
mod logging;
mod metrics;
mod not_found;
mod semaphore_service;
mod state;

#[get("/")]
async fn index() -> &'static str {
    "Hello from Throttle!"
}

#[actix_rt::main]
async fn main() -> io::Result<()> {
    let opt = Cli::from_args();

    logging::init().unwrap_or_else(|e| {
        eprintln!("Error during initialization of logging backend:\n{}", e);
    });

    let application_cfg = match application_cfg::ApplicationCfg::init(&opt.configuration) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "Couldn't parse {}.cfg:\n{}",
                opt.configuration.to_string_lossy(),
                e
            );
            return Ok(());
        }
    };

    info!("Hello From Throttle");

    // We only want to use one Map of semaphores across for all worker threads. To do this we wrap
    // it in `Data` which uses an `Arc` to share it between threads.
    let sem_map = Data::new(state::State::new(application_cfg.clone()));

    // Without this line, the metric is only going to be initalized, after the first request to an
    // unknown resource. I.e. We would see nothing instead of `num_404 0` in the metrics route,
    // before the first request to an unknown resource.
    not_found::initialize_metrics();

    HttpServer::new(move || {
        App::new()
            .data(application_cfg.clone())
            .app_data(sem_map.clone())
            .service(index)
            .service(health::health)
            .service(metrics::metrics)
            .service(favicon::favicon)
            .service(semaphore_service::acquire)
            .service(semaphore_service::remainder)
            .service(semaphore_service::release)
            .service(semaphore_service::wait_for_admission)
            .service(semaphore_service::remove_expired)
            .service(semaphore_service::put_lease)
            .default_service(
                // 404 for GET requests
                web::resource("").route(web::get().to(not_found::not_found)),
            )
    })
    .bind(&opt.endpoint())?
    .run()
    .await
}
