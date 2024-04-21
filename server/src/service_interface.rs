use axum::{routing::get, Router};
use std::{future::Future, io, sync::Arc};

use crate::{
    favicon::favicon, health::health, metrics::metrics, not_found::not_found,
    semaphore_service::semaphores, state::AppState, version::version,
};

pub trait ServiceInterface {
    fn run_service(&mut self, app_state: Arc<AppState>) -> impl Future<Output = io::Result<()>>;
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
            .route("/metrics", get(metrics))
            .merge(semaphores())
            .with_state(app_state.clone())
            // Stateless routes
            .route("/", get(index))
            .route("/health", get(health))
            .route("/favicon.ico", get(favicon))
            .route("/version", get(version))
            .fallback(not_found);

        let listener = tokio::net::TcpListener::bind(&self.endpoint).await?;
        axum::serve(listener, app).await
    }
}

async fn index() -> &'static str {
    "Hello from Throttle!"
}
