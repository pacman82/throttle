use log::{error, info};
use serde::Deserialize;
/// Stratosphere provides support for a configuartion file named `application.cfg`. This module sets
/// parsing it into an `ApplicationCfg` instance.
use std::{
    collections::HashMap,
    time::Duration,
    fs::File,
    io::{self, Read},
    path::Path,
};

pub type Semaphores = HashMap<String, i64>;

/// Representation of the `application.cfg` file passed to the service from stratosphere
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ApplicationCfg {
    #[serde(with = "humantime_serde", default = "ApplicationCfg::litter_collection_interval_default")]
    pub litter_collection_interval: Duration,
    #[serde(default = "HashMap::new")]
    pub semaphores: Semaphores,
}

impl Default for ApplicationCfg {
    fn default() -> ApplicationCfg {
        ApplicationCfg {
            litter_collection_interval: Duration::from_secs(300), // 5min
            semaphores: HashMap::new()
        }
    }
}

impl ApplicationCfg {
    // Set default of litter collection interval to 5min
    fn litter_collection_interval_default() -> Duration {
        ApplicationCfg::default().litter_collection_interval
    }

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

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn parse_toml_file() {
        let cfg = "litter_collection_interval = \"100ms\"\n\
                   \n\
                   [semaphores]\n\
                   A=1\n\
                   \n\
                ";
        let actual: ApplicationCfg = toml::from_str(cfg).unwrap();
        assert_eq!(actual.litter_collection_interval, Duration::from_millis(100));
        assert_eq!(actual.semaphores.get("A").unwrap(), &1);
    }

    /// Verify that the default configuration used in case of a missing file is identical to the
    /// configuration obtained from an empty toml file.
    #[test]
    fn default_configuration_equals_empty_configuration(){
        let empty : ApplicationCfg = toml::from_str("").unwrap();
        let default = ApplicationCfg::default();
        assert_eq!(empty, default);
    }
}
