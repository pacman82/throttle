use thiserror::Error;

/// Enumerates errors which can occur interacting with server state.
#[derive(Debug, Error, Clone, Copy)]
pub enum ThrottleError {
    #[error("Unknown semaphore")]
    UnknownSemaphore,
    #[error("Unknown peer")]
    UnknownPeer,
    #[error(
        "Acquiring lock would block forever. Lock asks for count {asked:?} yet full count is only \
        {max:?}."
    )]
    ForeverPending { asked: i64, max: i64 },
    #[error("May Deadlock. Due to violation of lock hierarchy.")]
    Deadlock,
    #[error("Already pending. Only one pendig lock per peer is allowed.")]
    AlreadyPending,
    #[error("Not Implemented.")]
    NotImplemented,
}
