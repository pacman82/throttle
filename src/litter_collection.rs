use crate::state::State;
use log::{debug, warn};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{spawn, JoinHandle},
    time::Duration,
};

/// Collects expired leases asynchronously. If all goes well leases are removed by the clients via
/// DELETE requests. Yet, clients crash and requests may never make it. To not leak semaphores in
/// such cases, the litter collection checks for expired leases in regular intervals and removes
/// them.
///
/// It does not have a drop handler joining the spawned thread. So if stop is not called at the end
/// of its lifetime the inner thread is deatched.
pub struct LitterCollection {
    // We currently make no use of the asynchronous executer and spawn a native system thread. In
    // would be nicer to reuse the Execute we use to drive the server requests, but advanced
    // asycronous abstractions are not quite there yet. For the time being we can afford one extra
    // thread.
    /// Used to wait for the next interval, or cancel execution of litter collection during waiting.
    stopped: Arc<(Mutex<bool>, Condvar)>,
    handle: JoinHandle<()>,
}

impl LitterCollection {
    pub fn stop(self) {
        // Tell litter collection thread to stop. This will cancel the wait between intervals.
        // Attention: Take care, to not hold the lock over join. This would cause a deadlock.
        *self.stopped.0.lock().unwrap() = true;
        self.stopped.1.notify_all();
        self.handle.join().unwrap();
    }
}

/// Starts a new thread that removes expired leases.
pub fn start(state: Arc<State>, interval: Duration) -> LitterCollection {
    let stopped = Arc::new((Mutex::new(false), Condvar::new()));
    // Copy of stopped for litter collecting thread
    let canceled = stopped.clone();
    let handle = spawn(move || {
        loop {
            let done = canceled.0.lock().unwrap();
            let done = {
                // Introduce extra scope here, so we do not hold lock to done during execution
                // of removed_expired.
                let (done, _wait_timeout_result) = canceled.1.wait_timeout(done, interval).unwrap();
                *done
            };
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
    LitterCollection { stopped, handle }
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
