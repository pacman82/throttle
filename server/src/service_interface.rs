use axum::{routing::get, Router};
use std::{io, time::Duration};
use tokio::{
    spawn,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    error::ThrottleError, favicon::favicon, health::health, leases::PeerId, metrics::metrics,
    not_found::not_found, semaphore_service::semaphores, state::Locks, version::version,
};

/// Channels used to communicate between request handlers and Domain logic
#[derive(Clone)]
pub struct Api {
    sender: mpsc::Sender<ServiceEvent>,
}

impl Api {
    pub fn new(sender: mpsc::Sender<ServiceEvent>) -> Self {
        Api { sender }
    }

    pub async fn new_peer(&mut self, expires_in: Duration) -> PeerId {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::NewPeer {
                answer_peer_id: send,
                expires_in,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn release_peer(&mut self, peer_id: PeerId) -> bool {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::ReleasePeer {
                answer_removed: send,
                peer_id,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn acquire(
        &mut self,
        peer_id: PeerId,
        semaphore: String,
        amount: i64,
        wait_for: Option<Duration>,
        expires_in: Option<Duration>,
    ) -> Result<bool, ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::AcquireLock {
                answer_acquired: send,
                peer_id,
                semaphore,
                amount,
                wait_for,
                expires_in,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn release(
        &mut self,
        peer_id: PeerId,
        semaphore: String,
    ) -> Result<(), ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::ReleaseLock {
                answer_release: send,
                peer_id,
                semaphore,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn is_acquired(&mut self, peer_id: PeerId) -> Result<bool, ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::IsAcquired {
                answer_is_aquired: send,
                peer_id,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn heartbeat(
        &mut self,
        peer_id: PeerId,
        expires_in: Duration,
    ) -> Result<(), ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::Heartbeat {
                peer_id,
                expires_in,
                answer_heartbeat: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn remainder(&mut self, semaphore: String) -> Result<i64, ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::Remainder {
                semaphore,
                answer_remainder: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn restore(
        &mut self,
        peer_id: PeerId,
        expires_in: Duration,
        acquired: Locks,
    ) -> Result<(), ThrottleError> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::Restore {
                peer_id,
                expires_in,
                acquired,
                answer_restore: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn update_metrics(&mut self) {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::UpdateMetrics {
                answer_update_metrics: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    pub async fn remove_expired(&mut self) -> usize {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::RemovedExpired {
                answer_remove_expired: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }
}

pub enum ServiceEvent {
    /// Create a new peer, with a given expiration time
    NewPeer {
        answer_peer_id: oneshot::Sender<PeerId>,
        expires_in: Duration,
    },
    /// Release Peer and all associated locks. `true` if peer did actually exist before the call.
    ReleasePeer {
        answer_removed: oneshot::Sender<bool>,
        peer_id: PeerId,
    },
    AcquireLock {
        answer_acquired: oneshot::Sender<Result<bool, ThrottleError>>,
        peer_id: PeerId,
        semaphore: String,
        amount: i64,
        wait_for: Option<Duration>,
        expires_in: Option<Duration>,
    },
    ReleaseLock {
        peer_id: PeerId,
        semaphore: String,
        answer_release: oneshot::Sender<Result<(), ThrottleError>>,
    },
    IsAcquired {
        peer_id: PeerId,
        answer_is_aquired: oneshot::Sender<Result<bool, ThrottleError>>,
    },
    Heartbeat {
        peer_id: PeerId,
        expires_in: Duration,
        answer_heartbeat: oneshot::Sender<Result<(), ThrottleError>>,
    },
    Remainder {
        semaphore: String,
        answer_remainder: oneshot::Sender<Result<i64, ThrottleError>>,
    },
    Restore {
        peer_id: PeerId,
        expires_in: Duration,
        acquired: Locks,
        answer_restore: oneshot::Sender<Result<(), ThrottleError>>,
    },
    UpdateMetrics {
        answer_update_metrics: oneshot::Sender<()>,
    },
    RemovedExpired {
        answer_remove_expired: oneshot::Sender<usize>,
    }
}

pub struct HttpServiceInterface {
    join_handle: JoinHandle<io::Result<()>>,
}

impl HttpServiceInterface {
    pub async fn new(endpoint: &str, api: Api) -> Result<Self, io::Error> {
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
        Ok(HttpServiceInterface {
            join_handle,
        })
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.join_handle.await.unwrap()
    }
}

async fn index() -> &'static str {
    "Hello from Throttle!"
}
