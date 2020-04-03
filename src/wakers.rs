use crate::{error::ThrottleError, leases::Leases};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

// State shared between Future and Wakers
struct Shared {
    result: Option<Result<(), ThrottleError>>,
    waker: Option<Waker>,
}

/// A Future which is completed, once all the pending locks of the peer are acquired.
///
/// One of these futures exists for each request pending in `block_unitl_acquired`.
struct FutureAcquired {
    shared: Arc<Mutex<Shared>>,
}

impl Future for FutureAcquired {
    type Output = Result<(), ThrottleError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.shared.lock().unwrap();
        match &shared.result {
            None => {
                if let &mut Some(ref mut waker) = &mut shared.waker {
                    // If a woker has been previously set, let's reuse the resources from the old
                    // one, rather than allocating a new one.
                    waker.clone_from(&mut cx.waker())
                } else {
                    shared.waker = Some(cx.waker().clone());
                }
                Poll::Pending
            }
            Some(_) => Poll::Ready(shared.result.take().unwrap()),
        }
    }
}

/// Wakers for `FutureAcquired` instances.
///
/// A waker is used to tell the task execute that a futures task may have proceeded and it is
/// sensible to poll them again. This one offers methods to make sure only Futures for the peers
/// those status may have changed get woken.
///
/// Usually there should be only one or zero requests per Peer, but we don't prevent multiple
/// request for a single peer, so we have to account for mulitple pending requests for a single
/// peer.
pub struct Wakers {
    wakers: Mutex<Vec<(u64, Weak<Mutex<Shared>>)>>,
}

impl Wakers {
    pub fn new() -> Self {
        Self {
            wakers: Mutex::new(Vec::new()),
        }
    }
    /// Future returns, once the peer has acquired its pending lock, or the peer gets removed.
    ///
    /// Attention: Do not call this method while holding a lock to `leases`.
    pub async fn all_acquired(
        &self,
        peer_id: u64,
        leases: &Mutex<Leases>,
    ) -> Result<(), ThrottleError> {
        let result = match leases.lock().unwrap().has_pending(peer_id) {
            Ok(false) => Some(Ok(())),
            Ok(true) => None,
            Err(e) => Some(Err(e)),
        };

        let strong = Arc::new(Mutex::new(Shared {
            result,
            waker: None,
        }));
        let weak = Arc::downgrade(&strong);
        {
            let mut wakers = self.wakers.lock().unwrap();
            wakers.retain(|(_peer, r)| r.strong_count() != 0);
            wakers.push((peer_id, weak));
        }
        FutureAcquired { shared: strong }.await
    }

    /// Resolves the pending futures
    ///
    /// * `peers`: Futures associated with these peers are resolved
    /// * `result`: The result these futures will return in their `.await` call
    pub fn resolve_with(&self, peers: &[u64], result: Result<(), ThrottleError>) {
        let mut wakers = self.wakers.lock().unwrap();
        for (peer, weak) in wakers.iter_mut() {
            if peers.contains(peer) {
                if let Some(strong) = weak.upgrade() {
                    let mut shared = strong.lock().unwrap();
                    if let Some(waker) = shared.waker.take() {
                        shared.result = Some(result);
                        waker.wake()
                    }
                }
            }
        }
    }
}
