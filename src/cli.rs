use std::path::PathBuf;
use structopt::StructOpt;

/// Arguments passed at the command line
#[derive(StructOpt)]
#[structopt(
    name = "Throttle",
    about = "A service providing semaphores for distributed systems."
)]
pub struct Cli {
    /// Address to bind to
    #[structopt(long = "address", default_value = "127.0.0.1")]
    pub address: String,
    /// Port on which the server listens to requests
    #[structopt(long = "port", default_value = "8000")]
    pub port: u16,
    /// Path to TOML configuration file
    #[structopt(long = "configuration", short = "c", default_value = "throttle.cfg")]
    pub configuration: PathBuf,
}

impl Cli {
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}
