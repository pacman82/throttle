use failure::{Error, Fail, ResultExt};
use gelf;
use log;
use env_logger;
use serde::Deserialize;
use serde_json;
use std::{fs::File, io};

/// Representation of the logging_config.json as placed by the stratosphere in the working directory
/// of the service. The types are structured like the JSON document:
///
/// ```json
/// {
///   handlers: {
///     gelf: {
///     }
///   }
/// }
/// ```
#[derive(Deserialize)]
struct LoggingConfig {
    handlers: Handler,
}

#[derive(Deserialize)]
struct Handler {
    gelf: GelfConfig,
}

#[derive(Deserialize)]
struct GelfConfig {
    /// E.g.: "westeurope-a.prototype.devel.throttle.throttle.x"
    #[serde(rename = "_name")]
    name: String,
    /// E.g.: "logging.a.westeurope.blue-yonder.cloud"
    host: String,
    /// E.g. "INFO" or "DEBUG"
    level: String,
    /// E.g. "12201"
    port: u16,
}

/// Initialize GELF logger if `logging_config.json` is found in the working directory.
pub fn init() -> Result<(), Error> {
    match File::open("logging_config.json") {
        Ok(file) => {
            let config = read_gelf_config(file).context("Error reading logging_config.json")?;

            let backend = gelf::UdpBackend::new((config.host.as_str(), config.port))
                .context("Error creating GELF UDP logging backend")?;
            let mut logger =
                gelf::Logger::new(Box::new(backend)).context("Error creating GELF logger.")?;
            logger.set_hostname(config.name);
            logger
                .install(map_log_level(config.level.as_str()))
                .context("Failed to install logger")?;
            Ok(())
        }
        Err(e) => {
            // Missing config file is fine and expected during local execution.
            if e.kind() == io::ErrorKind::NotFound {
                eprintln!("logging_config.json not found => Logging to stderr instead");
                env_logger::init();
                Ok(())
            } else {
                Err(e.context("Unable to open logging_config.json.").into())
            }
        }
    }
}

fn read_gelf_config(reader: impl io::Read) -> Result<GelfConfig, Error> {
    let config: LoggingConfig =
        serde_json::from_reader(reader).context("Error deserializing json")?;
    Ok(config.handlers.gelf)
}

fn map_log_level(level: &str) -> log::LevelFilter {
    match level {
        "INFO" => log::LevelFilter::Info,
        "DEBUG" => log::LevelFilter::Debug,
        unknown => {
            eprintln!(
                "WARNING: Found unknown log level {} in logging_config.json. Using 'INFO'",
                unknown
            );
            log::LevelFilter::Info
        }
    }
}
