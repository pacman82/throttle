use thiserror::Error;

/// Error returned by throttle client library.
#[derive(Error, Debug)]
pub enum Error {
    /// Errors of this kind should not be able to occur during normal operations. They hint at
    /// either coding error or a wrong configuration. An example would be requesting a lock to a
    /// non-existing semaphore.
    #[error("Throttle client domain error: {0}")]
    DomainError(String),
    /// The server did answer with something unexpected for a throttle servers.
    #[error(
        "Throttle client received a response not expected from a Throttle Server. Check server and \
        client version or maybe the Url to the throttle server is incorrect?"
    )]
    UnexpectedResponse,
    #[error("Throttle http client error: {0}")]
    Reqwest(#[from] reqwest::Error),
}
