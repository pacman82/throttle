use std::io;

use log::warn;
use tokio::{net::ToSocketAddrs, task::JoinHandle};

use crate::{configuration::Configuration, event_loop::EventLoop, http_shell::HttpShell};

/// Allows to run and shut down the application. Controls the application lifecycle, domain logic
/// and server.
pub struct App {
    event_loop_handle: JoinHandle<()>,
    service_interface: HttpShell,
}

impl App {
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
        let (event_loop, api) = EventLoop::new(application_cfg.semaphores);
        let event_loop_handle = event_loop.spawn();
        let service_interface = HttpShell::new(endpoint, api).await?;

        let app = App {
            event_loop_handle,
            service_interface,
        };
        Ok(app)
    }

    /// Gracefully shuts down the application and frees all associated resources.
    pub async fn shutdown(self) -> io::Result<()> {
        self.service_interface.shutdown().await?;
        self.event_loop_handle.await.unwrap();
        Ok(())
    }
}
