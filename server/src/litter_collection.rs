// Not detecting wether a bool is used to wait before a critical section is a known weakness of this
// See: https://rust-lang.github.io/rust-clippy/master/index.html#mutex_atomic
// We run into this here, so let's silence this lint for this file.
#![allow(clippy::mutex_atomic)]

use crate::state::AppState;
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::{
    spawn,
    sync::watch,
    task::JoinHandle,
    time::{timeout, Duration},
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
pub fn start(state: Arc<AppState>, interval: Duration) -> LitterCollection {
    let (send_stop, mut receive_stop) = watch::channel(false);
    info!("Start litter collection with interval: {:?}", interval);
    let handle = spawn(async move {
        loop {
            let done = timeout(interval, receive_stop.changed()).await.is_ok();
            if done {
                break;
            } else {
                let num_removed = state.remove_expired();
                if num_removed == 0 {
                    debug!("Litter collection did not find any expired leases.")
                } else {
                    warn!("Litter collection removed {} expired leases", num_removed);
                }
            }
        }
    });
    LitterCollection { send_stop, handle }
}

#[cfg(Debug)]
impl Drop for LitterCollection {
    fn drop(&mut self) {
        assert!(
            *self.stopped.0.lock().unwrap(),
            "Litter Collection has not been stopped before the end of its lifetime."
        )
    }
}
