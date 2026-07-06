//! Helper to start and stop a throttle server for each integration test.

use std::{
    io::{self, Write},
    process::ExitStatus,
    thread::sleep,
    time::Duration,
};
use tempfile::{NamedTempFile, TempPath};
use throttle_client::Client;
use tokio::{
    process::{Child, Command},
    time::timeout,
};

#[cfg(unix)]
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};

/// Helps us instantiate a throttle server in a child process in each test.
pub struct Server {
    /// The port our server instance is running on
    port: u16,
    /// Configuration file read by the throttle server. File is cleaned once TempPath is
    /// dropped.
    _config: TempPath,
    /// Child processes running the throttle server
    throttle_proc: Child,
}

impl Server {
    /// Since our tests run in parallel, we want to give each one its own server, with its own
    /// port.
    pub fn new(port: u16, config: &str) -> Server {
        // Write configuration into temporary file
        let mut config_file = NamedTempFile::new().unwrap();
        config_file.write_all(config.as_bytes()).unwrap();
        // Pass path to server process. File is cleaned once path is dropped.
        let config = config_file.into_temp_path();

        let throttle_proc = Command::new(env!("CARGO_BIN_EXE_throttle"))
            .args(["--port"])
            .arg(port.to_string())
            .arg("-c")
            .arg(&config)
            .spawn()
            .unwrap();

        // Give server process some time to boot, and be ready to take requests.
        sleep(Duration::from_millis(10));

        Server {
            port,
            _config: config,
            throttle_proc,
        }
    }

    pub fn make_client(&self) -> Client {
        Client::new(format!("http://localhost:{}", self.port)).unwrap()
    }

    /// Sends `SIGTERM` to the server process, as an orchestrator (e.g. a container runtime) would
    /// do to request a graceful shutdown.
    #[cfg(unix)]
    pub fn send_sigterm(&self) {
        let pid = Pid::from_raw(self.throttle_proc.id().expect("server must be running") as i32);
        signal::kill(pid, Signal::SIGTERM).unwrap();
    }

    /// Waits for the server process to exit, up to `duration`.
    pub async fn wait_for_termination(&mut self, duration: Duration) -> io::Result<ExitStatus> {
        timeout(duration, self.throttle_proc.wait()).await?
    }
}

/// Of course we want to clean up, after each test is done. So we implement drop.
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.throttle_proc.start_kill();
    }
}
