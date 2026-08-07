use std::io;

use log::warn;
use tokio::net::ToSocketAddrs;

use crate::{
    configuration::Configuration, http_shell::HttpShell, semaphore_runtime::SemaphoreRuntime,
};

/// Allows to run and shut down the application. Controls the application lifecycle, domain logic
/// and server.
pub struct Throttle {
    semaphores: SemaphoreRuntime,
    service_interface: HttpShell,
}

impl Throttle {
    /// Constructs the application including http interface. Both the http interface and the event
    /// loop are already running in the background once this future completes, i.e. the
    /// application is fully able to answer requests. This allows for testing without sleep
    /// relying on timings.
    pub async fn new(
        application_cfg: Configuration,
        endpoint: impl ToSocketAddrs,
    ) -> io::Result<Self> {
        if application_cfg.semaphores.is_empty() {
            warn!("No semaphores configured.")
        }
        let semaphores = SemaphoreRuntime::new(application_cfg.semaphores);
        let service_interface = HttpShell::new(endpoint, semaphores.client()).await?;

        let app = Throttle {
            semaphores,
            service_interface,
        };
        Ok(app)
    }

    /// Gracefully shuts down the application and frees all associated resources.
    pub async fn shutdown(self) -> io::Result<()> {
        self.service_interface.shutdown().await?;
        self.semaphores.shutdown().await;
        Ok(())
    }
}
