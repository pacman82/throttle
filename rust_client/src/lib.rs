//! Client for Throttle. Throttle is a http semaphore service, providing semaphores for distributed
//! systems.
mod error;
mod client;

pub use error::Error;
pub use client::Client;