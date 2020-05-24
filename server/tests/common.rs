//! Helper to start and stop a throttle server for each integration test.

use tempfile::{NamedTempFile, TempPath};
use std::{io::Write, process::{Child, Command}, thread::sleep, time::Duration};
use throttle_client::Client;

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
            .args(&["--port"])
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
        Client::new(&format!("http://localhost:{}", self.port)).unwrap()
    }
}

/// Of course we want to clean up, after each test is done. So we implement drop.
impl Drop for Server {
    fn drop(&mut self) {
        self.throttle_proc.kill().unwrap();
    }
}
