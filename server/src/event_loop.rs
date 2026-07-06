use crate::{
    configuration::Semaphores,
    error::ThrottleError,
    leases::{PeerDescription, PeerId},
    state::{AppState, Locks},
};
use std::{future::pending, time::Duration};
use tokio::{
    select, spawn,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::sleep_until,
};

pub struct EventLoop {
    event_receiver: mpsc::Receiver<ServiceEvent>,
    app_state: AppState,
}

impl EventLoop {
    /// Constructs the event loop together with the one `Api` handle used to send it events.
    pub fn new(semaphores: Semaphores) -> (Self, Api) {
        let app_state = AppState::new(semaphores);
        let (sender, event_receiver) = mpsc::channel(5);
        let api = Api::new(sender);
        let event_loop = EventLoop {
            event_receiver,
            app_state,
        };
        (event_loop, api)
    }

    /// Runs the event loop on its own green thread which resolves once every `Api` handle has been
    /// dropped.
    pub fn spawn(self) -> JoinHandle<()> {
        spawn(self.run())
    }

    pub async fn run(mut self) {
        loop {
            let min_valid_until = self.app_state.min_valid_until();
            let sleep_until_lease_expires = async {
                if let Some(valid_until) = min_valid_until {
                    sleep_until(valid_until.into()).await;
                } else {
                    pending::<()>().await;
                }
            };
            select! {
                // This branch handles any incoming events, returned leases, requests for new
                // leases, heartbeats, etc. `None` means all `Api` handles have been dropped, so
                // we exit the event loop.
                event = self.event_receiver.recv() => {
                    match event {
                        Some(event) => self.handle_event(event),
                        None => break,
                    }
                }
                // This branch handles the litter collection. We need to remove expired leases.
                // Leases typically expire if the client does not explicitly delete them and also
                // does not extend their lifetime using heartbeats.
                () = sleep_until_lease_expires => {
                    self.app_state.remove_expired();
                }
            }
        }
    }

    pub fn handle_event(&mut self, event: ServiceEvent) {
        match event {
            ServiceEvent::NewPeer {
                answer_peer_id,
                expires_in,
            } => {
                let peer_id = self.app_state.new_peer(expires_in);
                let _ = answer_peer_id.send(peer_id);
            }
            ServiceEvent::ReleasePeer {
                answer_removed,
                peer_id,
            } => {
                let removed = self.app_state.release(peer_id);
                let _ = answer_removed.send(removed);
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
                    let _ = answer_acquired.send(acquired);
                });
            }
            ServiceEvent::ReleaseLock {
                peer_id,
                semaphore,
                answer_release,
            } => {
                let result = self.app_state.release_lock(peer_id, &semaphore);
                let _ = answer_release.send(result);
            }
            ServiceEvent::IsAcquired {
                peer_id,
                answer_is_aquired,
            } => {
                let result = self.app_state.is_acquired(peer_id);
                let _ = answer_is_aquired.send(result);
            }
            ServiceEvent::Heartbeat {
                peer_id,
                expires_in,
                answer_heartbeat,
            } => {
                let result = self.app_state.heartbeat(peer_id, expires_in);
                let _ = answer_heartbeat.send(result);
            }
            ServiceEvent::Remainder {
                semaphore,
                answer_remainder,
            } => {
                let result = self.app_state.remainder(&semaphore);
                let _ = answer_remainder.send(result);
            }
            ServiceEvent::Restore {
                peer_id,
                expires_in,
                acquired,
                answer_restore,
            } => {
                let result = self.app_state.restore(peer_id, expires_in, &acquired);
                let _ = answer_restore.send(result);
            }
            ServiceEvent::UpdateMetrics {
                answer_update_metrics,
            } => {
                self.app_state.update_metrics();
                let _ = answer_update_metrics.send(());
            }
            ServiceEvent::ListPeers { answer_list_peers } => {
                let list_of_peers = self.app_state.list_of_peers();
                let _ = answer_list_peers.send(list_of_peers);
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
    fn new(sender: mpsc::Sender<ServiceEvent>) -> Self {
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
    async fn list_of_peers(&mut self) -> Vec<PeerDescription>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    /// Once every `Api` handle is dropped, the event loop is expected to notice the channel has
    /// closed and terminate `run`.
    #[tokio::test]
    async fn run_terminates_after_last_sender_is_dropped() {
        let (event_loop, api) = EventLoop::new(Semaphores::new());

        // The only handle is dropped immediately, before `run` is even called.
        drop(api);

        let result = timeout(Duration::from_millis(500), event_loop.run()).await;

        assert!(
            result.is_ok(),
            "event loop did not terminate after all external senders were dropped"
        );
    }
}
