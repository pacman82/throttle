use crate::state::AppState;
use log::{debug, warn};
use std::sync::Arc;
use tokio::{
    select, spawn,
    sync::watch,
    task::JoinHandle,
    time::{sleep_until, Instant},
};

/// Collects expired leases asynchronously. If all goes well leases are removed by the clients via
/// DELETE requests. Yet, clients crash and requests may never make it. To not leak semaphores in
/// such cases, the litter collection checks for expired leases in regular intervals and removes
/// them.
///
/// It does not have a drop handler joining the spawned thread. So if stop is not called at the end
/// of its lifetime the inner thread is deatched.
pub struct LitterCollection {
    /// Used to cancel execution of litter collection
    send_stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl LitterCollection {
    pub async fn stop(self) {
        // Tell litter collection thread to stop. This will cancel the wait between intervals.
        // Attention: Take care, to not hold the lock over join. This would cause a deadlock.
        self.send_stop.send(true).unwrap();
        self.handle.await.unwrap();
    }
}

/// Starts a new thread that removes expired leases.
pub fn start(state: Arc<AppState>) -> LitterCollection {
    let mut watch_valid_until = state.subscribe_valid_until();
    let mut maybe_valid_until = *watch_valid_until.borrow_and_update();
    let (send_stop, mut watch_stop) = watch::channel(false);
    let handle = spawn(async move {
        loop {
            if let Some(valid_until) = maybe_valid_until {
                // We know then the next peer would expire given that there is no heartbeat
                select! {
                    _ = sleep_until(Instant::from_std(valid_until)) => (),
                    _ = watch_stop.changed() => break,
                    _ = watch_valid_until.changed() => {
                            maybe_valid_until = *watch_valid_until.borrow_and_update()
                    }
                }
            } else {
                // There are no peers. Let's wait for this to change, or the application to stop
                select! {
                    _ = watch_stop.changed() => break,
                    _ = watch_valid_until.changed() => {
                            maybe_valid_until = *watch_valid_until.borrow_and_update()
                    }
                }
            }
            let num_removed = state.remove_expired();
            if num_removed == 0 {
                debug!("Litter collection did not find any expired leases.")
            } else {
                warn!("Litter collection removed {} expired leases", num_removed);
            }
        }
    });
    LitterCollection { send_stop, handle }
}
