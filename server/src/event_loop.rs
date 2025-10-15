use crate::{
    application_cfg::Semaphores,
    error::ThrottleError,
    leases::{PeerDescription, PeerId},
    state::{AppState, Locks},
};
use std::time::{Duration, Instant};
use tokio::{
    spawn,
    sync::{mpsc, oneshot, watch},
};

pub struct EventLoop {
    event_receiver: mpsc::Receiver<ServiceEvent>,
    api: Api,
    /// Used to tell litter collection when the next lease is going to expire (given it is not
    /// prolonged using a heartbeat). We send `None` if there are no active leases.
    send_min_valid_until: watch::Sender<Option<Instant>>,
    app_state: AppState,
}

impl EventLoop {
    pub fn new(semaphores: Semaphores) -> Self {
        let app_state = AppState::new(semaphores);
        let (sender, event_receiver) = mpsc::channel(5);
        let api = Api::new(sender);
        EventLoop {
            event_receiver,
            api,
            app_state,
            send_min_valid_until: watch::Sender::new(None),
        }
    }

    /// Provides an Api to interfaces and actors, in order to send events to the application logic.
    pub fn api(&self) -> Api {
        self.api.clone()
    }

    /// Allows the litter collection to watch for the earliest anticipated expiration of a leak.
    /// The watched timepoint will be updated if it changes. It can become even earlier or (more
    /// likely is prolonged). `None` implies there are no leases which can expire.
    pub fn watch_valid_until(&self) -> watch::Receiver<Option<Instant>> {
        self.send_min_valid_until.subscribe()
    }

    pub async fn run(mut self) {
        while let Some(event) = self.event_receiver.recv().await {
            match event {
                ServiceEvent::NewPeer {
                    answer_peer_id,
                    expires_in,
                } => {
                    let peer_id = self.app_state.new_peer(expires_in);
                    answer_peer_id.send(peer_id).unwrap();
                }
                ServiceEvent::ReleasePeer {
                    answer_removed,
                    peer_id,
                } => {
                    let removed = self.app_state.release(peer_id);
                    answer_removed.send(removed).unwrap();
                }
                ServiceEvent::AcquireLock {
                    answer_acquired,
                    peer_id,
                    semaphore,
                    amount,
                    wait_for,
                    expires_in,
                } => {
                    let acquired_future = self
                        .app_state
                        .acquire(peer_id, semaphore, amount, wait_for, expires_in);
                    spawn(async move {
                        let acquired = acquired_future.await;
                        answer_acquired.send(acquired).unwrap()
                    });
                }
                ServiceEvent::ReleaseLock {
                    peer_id,
                    semaphore,
                    answer_release,
                } => {
                    let result = self.app_state.release_lock(peer_id, &semaphore);
                    answer_release.send(result).unwrap();
                }
                ServiceEvent::IsAcquired {
                    peer_id,
                    answer_is_aquired,
                } => {
                    let result = self.app_state.is_acquired(peer_id);
                    answer_is_aquired.send(result).unwrap();
                }
                ServiceEvent::Heartbeat {
                    peer_id,
                    expires_in,
                    answer_heartbeat,
                } => {
                    let result = self.app_state.heartbeat(peer_id, expires_in);
                    answer_heartbeat.send(result).unwrap();
                }
                ServiceEvent::Remainder {
                    semaphore,
                    answer_remainder,
                } => {
                    let result = self.app_state.remainder(&semaphore);
                    answer_remainder.send(result).unwrap()
                }
                ServiceEvent::Restore {
                    peer_id,
                    expires_in,
                    acquired,
                    answer_restore,
                } => {
                    let result = self.app_state.restore(peer_id, expires_in, &acquired);
                    answer_restore.send(result).unwrap();
                }
                ServiceEvent::UpdateMetrics {
                    answer_update_metrics,
                } => {
                    self.app_state.update_metrics();
                    answer_update_metrics.send(()).unwrap()
                }
                ServiceEvent::RemovedExpired {
                    answer_remove_expired,
                } => {
                    let num_expired = self.app_state.remove_expired();
                    answer_remove_expired.send(num_expired).unwrap();
                }
                ServiceEvent::ListPeers { answer_list_peers } => {
                    let list_of_peers = self.app_state.list_of_peers();
                    answer_list_peers.send(list_of_peers).unwrap();
                }
            }
            if *self.send_min_valid_until.borrow() != self.app_state.min_valid_until() {
                let _ = self
                    .send_min_valid_until
                    .send(self.app_state.min_valid_until());
            }
        }
    }
}

/// Channels used to communicate between request handlers and Domain logic
#[derive(Clone)]
pub struct Api {
    sender: mpsc::Sender<ServiceEvent>,
}

impl Api {
    pub fn new(sender: mpsc::Sender<ServiceEvent>) -> Self {
        Api { sender }
    }
}

impl SemaphoresApi for Api {
    async fn new_peer(&mut self, expires_in: Duration) -> PeerId {
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

    async fn release_peer(&mut self, peer_id: PeerId) -> bool {
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

    async fn acquire(
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

    async fn release(&mut self, peer_id: PeerId, semaphore: String) -> Result<(), ThrottleError> {
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

    async fn is_acquired(&mut self, peer_id: PeerId) -> Result<bool, ThrottleError> {
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

    async fn heartbeat(
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

    async fn remainder(&mut self, semaphore: String) -> Result<i64, ThrottleError> {
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

    async fn restore(
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

    async fn update_metrics(&mut self) {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::UpdateMetrics {
                answer_update_metrics: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    async fn remove_expired(&mut self) -> usize {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::RemovedExpired {
                answer_remove_expired: send,
            })
            .await
            .unwrap();
        recv.await.unwrap()
    }

    async fn list_of_peers(&mut self) -> Vec<PeerDescription> {
        let (send, recv) = oneshot::channel();
        self.sender
            .send(ServiceEvent::ListPeers {
                answer_list_peers: send,
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
    },
    ListPeers {
        answer_list_peers: oneshot::Sender<Vec<PeerDescription>>,
    },
}

pub trait SemaphoresApi {
    async fn new_peer(&mut self, expires_in: Duration) -> PeerId;
    async fn release_peer(&mut self, peer_id: PeerId) -> bool;
    async fn acquire(
        &mut self,
        peer_id: PeerId,
        semaphore: String,
        amount: i64,
        wait_for: Option<Duration>,
        expires_in: Option<Duration>,
    ) -> Result<bool, ThrottleError>;
    async fn release(&mut self, peer_id: PeerId, semaphore: String) -> Result<(), ThrottleError>;
    async fn is_acquired(&mut self, peer_id: PeerId) -> Result<bool, ThrottleError>;
    async fn heartbeat(
        &mut self,
        peer_id: PeerId,
        expires_in: Duration,
    ) -> Result<(), ThrottleError>;
    async fn remainder(&mut self, semaphore: String) -> Result<i64, ThrottleError>;
    async fn restore(
        &mut self,
        peer_id: PeerId,
        expires_in: Duration,
        acquired: Locks,
    ) -> Result<(), ThrottleError>;
    async fn update_metrics(&mut self);
    async fn remove_expired(&mut self) -> usize;
    async fn list_of_peers(&mut self) -> Vec<PeerDescription>;
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::*;

    #[tokio::test]
    async fn announce_change_in_valid_until_through_new_peer() {
        // Given an application state with no peers, the first call to acquire should announce that
        // now there is something to expire.
        let semaphores = Semaphores::new();
        let app = EventLoop::new(semaphores);
        let mut listener = app.watch_valid_until();
        let mut api = app.api();
        spawn(app.run());

        // When waiting for the peer to expire
        let one_sec = Duration::from_secs(1);
        let _peer_id = api.new_peer(one_sec).await;

        // Then
        assert!(listener.borrow_and_update().is_some())
    }

    #[tokio::test]
    async fn announce_change_in_valid_until_through_restored_peer() {
        // Given
        let semaphores = Semaphores::new();
        let app = EventLoop::new(semaphores);
        let mut listener = app.watch_valid_until();
        let mut api = app.api();
        spawn(app.run());

        // When
        let one_sec = Duration::from_secs(1);
        let _peer_id = api.restore(1, one_sec, HashMap::new()).await;

        // Then
        assert!(listener.borrow_and_update().is_some())
    }

    #[tokio::test]
    async fn announce_change_in_min_valid_until_through_removed_peer() {
        // Given
        let semaphores = Semaphores::new();
        let app = EventLoop::new(semaphores);
        let mut listener = app.watch_valid_until();
        let mut api = app.api();
        spawn(app.run());
        let peer_id = api.new_peer(Duration::from_secs(1)).await;

        // When
        api.release_peer(peer_id).await;

        // Then
        assert!(listener.borrow_and_update().is_none())
    }

    #[tokio::test]
    async fn announce_change_in_min_valid_until_through_expired_peer() {
        // Given
        let semaphores = Semaphores::new();
        let app = EventLoop::new(semaphores);
        let mut listener = app.watch_valid_until();
        let mut api = app.api();
        spawn(app.run());
        let _ = api.new_peer(Duration::from_secs(1)).await;

        // When creating a new peer and letting it expiring before the first one
        let old_valid_until = *listener.borrow_and_update();
        let _ = api.new_peer(Duration::from_secs(0)).await;
        api.remove_expired().await;

        // Then
        let after_expiration = *listener.borrow_and_update();
        assert_eq!(old_valid_until, after_expiration)
    }
}
