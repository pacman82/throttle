use thiserror::Error;

/// Enumerates errors which can occur interacting with server state.
#[derive(Debug, Error)]
pub enum ThrottleError {
    #[error("Unknown semaphore")]
    UnknownSemaphore,
    #[error("Unknown peer")]
    UnknownPeer,
    #[error("Acquiring lock would block forever. Lock asks for count {asked:?} yet full count is \
            only {max:?}.")]
    ForeverPending { asked: i64, max: i64 },
}