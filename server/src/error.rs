use thiserror::Error;

/// Enumerates errors which can occur interacting with server state.
#[derive(Debug, Error, Clone, Copy)]
pub enum ThrottleError {
    #[error("Unknown semaphore")]
    UnknownSemaphore,
    #[error("Unknown peer")]
    UnknownPeer,
    #[error(
        "Lock can never be acquired. Lock asks for count {asked:?} yet full count is only {max:?}."
    )]
    Never { asked: i64, max: i64 },
    #[error(
        "Lock hierachy violation. This may deadlock. The current lock level is {current:?} the
        requested lock level was {requested:?}."
    )]
    Deadlock { current: i32, requested: i32 },
    #[error("Already pending. Only one pendig lock per peer is allowed.")]
    AlreadyPending,
    #[error("Lock count must be a positive number. Found: {count:?}.")]
    InvalidLockCount { count: i64 },
    #[error("Restore is not allowed to change existing peers.")]
    ChangeThroughRestore,
    #[error("Shrinking the count of an existing lock is currently not implemented.")]
    ShrinkingLockCount,
}
