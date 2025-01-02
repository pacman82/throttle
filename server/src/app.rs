use std::io;

use log::warn;
use tokio::net::ToSocketAddrs;

use crate::{
    application_cfg::ApplicationCfg,
    event_loop::EventLoop,
    litter_collection::{self, LitterCollection},
    service_interface::HttpServiceInterface,
};

/// Allows to initialize and run the application. Most importantly the separation of [`App::new`]
/// and [`App::run`], allows for easier testing, because we can now explicitly wait for the service
/// to be able to accept incoming requests (i.e. the port is bound to a listener), without relying
/// on sleep timings.
pub struct App {
    event_loop: EventLoop,
    litter_collection: LitterCollection,
    service_interface: HttpServiceInterface,
}

impl App {
    /// Constructs the application including http interface. Application will accept request, once
    /// the future is completed, but it will only actually answer them once `run` is executed.
    pub async fn new(
        application_cfg: ApplicationCfg,
        endpoint: impl ToSocketAddrs,
    ) -> io::Result<Self> {
        if application_cfg.semaphores.is_empty() {
            warn!("No semaphores configured.")
        }
        let event_loop = EventLoop::new(application_cfg.semaphores);
        let service_interface = HttpServiceInterface::new(endpoint, event_loop.api()).await?;

        // Removes expired peers asynchrounously. We must take care not to exit early with the
        // ?-operator in order to not be left with a detached thread.
        let litter_collection =
            litter_collection::start(event_loop.watch_valid_until(), event_loop.api());

        let app = App {
            event_loop,
            litter_collection,
            service_interface,
        };
        Ok(app)
    }

    /// Runs application to completion and frees all associated resources.
    pub async fn run(self) -> io::Result<()> {
        self.event_loop.run().await;

        // Don't use ? to early return before stopping the lc.
        let result = self.service_interface.shutdown().await;

        // Stop litter collection.
        self.litter_collection.stop().await;

        result
    }
}
