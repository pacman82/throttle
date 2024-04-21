use axum::{routing::get, Router};
use std::{collections::HashMap, future::Future, io, sync::Arc};
use tokio::{spawn, sync::mpsc, task::JoinHandle};

use crate::{
    application_cfg::SemaphoreCfg, favicon::favicon, health::health, metrics::metrics,
    not_found::not_found, semaphore_service::semaphores, state::AppState, version::version,
};

pub trait ServiceInterface {
    fn app_state(&self) -> Arc<AppState>;
    fn shutdown(self) -> impl Future<Output = io::Result<()>>;
    fn event(&mut self) -> impl Future<Output = Option<ServiceEvent>>;
}

pub enum ServiceEvent {
    NewPeer,
}

pub struct HttpServiceInterface {
    app_state: Arc<AppState>,
    event_receiver: mpsc::Receiver<ServiceEvent>,
    join_handle: JoinHandle<io::Result<()>>,
}

impl HttpServiceInterface {
    pub async fn new(
        semaphores_cfg: HashMap<String, SemaphoreCfg>,
        endpoint: &str,
    ) -> Result<Self, io::Error> {
        let (sender, event_receiver) = mpsc::channel(10);
        // We only want to use one Map of semaphores across all worker threads. To do this we wrap it in
        // an `Arc` to share it between threads.
        let app_state = Arc::new(AppState::new(semaphores_cfg));

        // TODO: idea: introduce move route for new_peers here and make it dependend on `sender`
        // instead of `app_state`. It will then forward the request to the application via event.
        // Initially we can try to answer with a OneShot channel

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

        let listener = tokio::net::TcpListener::bind(endpoint).await?;
        let join_handle = spawn(async move { axum::serve(listener, app).await });
        Ok(HttpServiceInterface {
            app_state,
            event_receiver,
            join_handle,
        })
    }
}

impl ServiceInterface for HttpServiceInterface {
    async fn event(&mut self) -> Option<ServiceEvent> {
        self.event_receiver.recv().await
    }

    async fn shutdown(self) -> io::Result<()> {
        self.join_handle.await.unwrap()
    }

    fn app_state(&self) -> Arc<AppState> {
        self.app_state.clone()
    }
}

async fn index() -> &'static str {
    "Hello from Throttle!"
}
