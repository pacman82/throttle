//! This module defines the `/metrics` route with which metrics can be accessed from via http. It
//! does and should not define individual metrics. These should go into their respective modules.

use crate::service_interface::Api;
use axum::extract::State;
use prometheus::{Encoder, TextEncoder};

/// Renders the default prometheus registry into text
pub async fn metrics(mut api: State<Api>) -> String {
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();

    // Rather than updating the Metrics with every state change it is probably saner to gather the
    // metrics in bulk for each Request to the `metrics` endpoint.
    api.update_metrics().await;

    encoder.encode(&prometheus::gather(), &mut buf).unwrap();
    String::from_utf8(buf).expect("Prometheus encoder should always return valid utf8")
}
