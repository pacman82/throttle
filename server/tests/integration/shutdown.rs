//! Verifies that the server shuts down gracefully (and quickly) once it receives `SIGTERM`, as an
//! orchestrator (e.g. a container runtime) would send when stopping the service.

use crate::common::Server;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[tokio::test]
async fn server_finished_with_success_status_code_after_terminate() {
    // Given a running server
    let config = "[semaphores]\nA=1\n";
    let mut server = Server::new(8090, config);

    // When sending SIGTERM to the server process
    server.send_sigterm();
    // And waiting for it to finish
    let status = server
        .wait_for_termination(Duration::from_secs(5))
        .await
        .unwrap();

    // Then it should have finished with a success status code (`0`)
    assert!(status.success());
}

#[cfg(unix)]
#[tokio::test]
async fn server_shuts_down_within_1_sec() {
    // Given a running server
    let config = "[semaphores]\nA=1\n";
    let mut server = Server::new(8091, config);

    // When sending SIGTERM to the server process
    server.send_sigterm();
    // And measuring the time it takes to shut down
    let start = Instant::now();
    server
        .wait_for_termination(Duration::from_secs(5))
        .await
        .unwrap();
    let elapsed = start.elapsed();

    // Then it should have taken less than 1 second to shut down
    assert!(
        elapsed <= Duration::from_secs(1),
        "shutdown took {elapsed:?}"
    );
}
