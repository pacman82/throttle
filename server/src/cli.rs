use std::path::PathBuf;
use clap::Parser;

/// Arguments passed at the command line
#[derive(Parser)]
#[clap(
    name = "Throttle",
    about = "A service providing semaphores for distributed systems."
)]
pub struct Cli {
    /// Address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub address: String,
    /// Port on which the server listens to requests
    #[arg(long, default_value = "8000")]
    pub port: u16,
    /// Path to TOML configuration file
    #[arg(long, short = 'c', default_value = "throttle.toml")]
    pub configuration: PathBuf,
}

impl Cli {
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}
