use serde::Deserialize;

/// Controls logging behaviour of throttle. Set via the configuration file
#[derive(Deserialize, Default, PartialEq, Eq, Clone, Debug)]
pub struct LoggingConfig {
    #[serde(default)]
    pub stderr: StdErrConfig,
}

#[derive(Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct StdErrConfig {
    /// E.g. "INFO" or "DEBUG"
    pub level: String,
}

impl Default for StdErrConfig {
    fn default() -> Self {
        StdErrConfig {
            level: "WARN".to_string(),
        }
    }
}

/// Initialize GELF logger if `logging_config.json` is found in the working directory.
pub fn init(config: &LoggingConfig) {
    let environment =
        env_logger::Env::default().filter_or("THROTTLE_LOG", config.stderr.level.as_str());
    env_logger::Builder::from_env(environment).init();
}