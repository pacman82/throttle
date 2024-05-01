use crate::{
    state::AppState,
    service_interface::{Api, ServiceEvent},
    application_cfg::Semaphores
};
use std::time::Instant;
use tokio::{spawn, sync::{mpsc, watch}};

pub struct EventLoop {
    event_receiver: mpsc::Receiver<ServiceEvent>,
    api: Api,
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
        self.app_state.watch_valid_until()
    }

    pub async fn run_event_loop(mut self) {
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
            }
        }
    }
}