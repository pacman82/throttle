use axum::{Router, routing::get};
use std::io;
use tokio::{net::ToSocketAddrs, spawn, sync::watch, task::JoinHandle};

use crate::{
    event_loop::Api, favicon::favicon, health::health, metrics::metrics, not_found::not_found,
    semaphore_shell::semaphores, version::version,
};

pub struct HttpShell {
    /// Indicates whether the server is about to shut down. Setting this to `true` triggers graceful
    /// shutdown, which stops accepting new connections and waits for in-flight requests to finish.
    shutting_down: watch::Sender<bool>,
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
        let (shutting_down, mut shutting_down_receiver) = watch::channel(false);
        let join_handle = spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutting_down_receiver
                        .wait_for(|&is_shutting_down| is_shutting_down)
                        .await;
                })
                .await
        });
        Ok(HttpShell {
            shutting_down,
            join_handle,
        })
    }

    /// Gracefully shuts down the http server. Waits for in-flight requests to finish, but stops
    /// accepting new connections.
    pub async fn shutdown(self) -> io::Result<()> {
        let _ = self.shutting_down.send(true);
        self.join_handle.await.unwrap()
    }
}

async fn index() -> &'static str {
    "Hello from Throttle!"
}
