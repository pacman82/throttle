//! This module defines the `/metrics` route with which metrics can be accessed from via http. It
//! does and should not define individual metrics. These should go into their respective modules.
//! See the 404 route in the `not_found` module as an example.

use actix_web::get;
use prometheus::{Encoder, TextEncoder};

/// Renders the default prometheus registry into text
#[get("/metrics")]
pub async fn metrics() -> String {
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();

    encoder.encode(&prometheus::gather(), &mut buf).unwrap();
    String::from_utf8(buf).expect("Prometheus encoder should always return valid utf8")
}
