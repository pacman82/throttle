use log::{error, info};
use serde::Deserialize;
/// Stratosphere provides support for a configuartion file named `application.cfg`. This module sets
/// parsing it into an `ApplicationCfg` instance.
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::Path,
};

/// Representation of the `application.cfg` file passed to the service from stratosphere
#[derive(Deserialize, Debug, Default, Clone)]
pub struct ApplicationCfg {
    pub semaphores: HashMap<String, i64>,
}

impl ApplicationCfg {
    /// Checks for a file named `application.cfg` in the working directory. It is then used to
    /// create a new configuration. If the file can not be found a default configuration is created.
    pub fn init(path: &Path) -> Result<ApplicationCfg, io::Error> {
        match File::open(path) {
            Ok(mut file) => {
                let mut buffer = String::new();
                file.read_to_string(&mut buffer)?;
                let cfg = toml::from_str(&buffer)?;
                info!("Use configuration from {}", path.to_string_lossy());
                Ok(cfg)
            }
            Err(e) => {
                // Missing config file is fine and expected during local execution.
                if e.kind() == io::ErrorKind::NotFound {
                    info!(
                        "{} not found => Using empty default configuration.",
                        path.to_string_lossy()
                    );
                    Ok(ApplicationCfg::default())
                } else {
                    let err = e;
                    error!("{}", err);
                    Err(err)
                }
            }
        }
    }
}
