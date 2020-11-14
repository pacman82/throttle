//! Client for Throttle. Throttle is a http semaphore service, providing semaphores for distributed
//! systems.
mod client;
mod error;

pub use client::Client;
pub use error::Error;
