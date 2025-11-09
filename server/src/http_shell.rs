use axum::{Router, routing::get};
use std::io;
use tokio::{net::ToSocketAddrs, spawn, task::JoinHandle};

use crate::{
    event_loop::Api, favicon::favicon, health::health, metrics::metrics, not_found::not_found,
    semaphore_shell::semaphores, version::version,
};

pub struct HttpShell {
    join_handle: JoinHandle<io::Result<()>>,
}

impl HttpShell {
    pub async fn new(endpoint: impl ToSocketAddrs, api: Api) -> Result<Self, io::Error> {
        let app: Router = Router::new()
            .route("/metrics", get(metrics))
            .merge(semaphores())
            .with_state(api)
            // Stateless routes
            .route("/", get(index))
            .route("/health", get(health))
            .route("/favicon.ico", get(favicon))
            .route("/version", get(version))
            .fallback(not_found);

        let listener = tokio::net::TcpListener::bind(endpoint).await?;
        let join_handle = spawn(async move { axum::serve(listener, app).await });
        Ok(HttpShell { join_handle })
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.join_handle.await.unwrap()
    }
}

async fn index() -> &'static str {
    "Hello from Throttle!"
}
