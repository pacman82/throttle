//! Application configuration, and how it is read from a TOML file.

use crate::logging::LoggingConfig;
use serde::{Deserialize, de};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::Path,
};
use thiserror::Error;

/// Error scenarious which may occurr then reading the configuration.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Unable to read configuration file. {0}")]
    ReadConfigFile(#[source] io::Error),
    #[error("Unable to deserilize configuration. {0}")]
    DeserilizeToml(#[source] toml::de::Error),
}

/// Configuration for one Semaphore
///
/// The `Deserialize` trait is not derived, but manually implemented. This is so to make it possible
/// to have a simple and a verbose representations in Toml for semaphores.
///
/// *Simple*:
///
/// ```toml
/// [semaphores]
/// A = 42
/// ```
///
/// *Verbose*
///
/// ```toml
/// [semaphores]
/// A = { count : 42 }
/// ```
///
/// ```toml
/// [semaphores.A]
/// count = 42
/// ```
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemaphoreCfg {
    pub max: i64,
    /// While holding a mutex at level N one may only acquire mutices at lower levels.
    pub level: i32,
}

impl<'de> de::Deserialize<'de> for SemaphoreCfg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct SemaphoreVisitor;

        /// Repetition of Semaphore, but with derived `Deserialize` Trait.
        #[derive(Deserialize)]
        pub struct Verbose {
            max: i64,
            #[serde(default)]
            level: i32,
        }

        impl<'de> de::Visitor<'de> for SemaphoreVisitor {
            type Value = SemaphoreCfg;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "a semaphore count like 42 or a verbose semaphore configuration like \
                    { max = 42 }",
                )
            }

            fn visit_i64<E>(self, i: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SemaphoreCfg { max: i, level: 0 })
            }

            fn visit_map<V>(self, map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mvd = de::value::MapAccessDeserializer::new(map);
                Verbose::deserialize(mvd).map(|Verbose { max, level }| SemaphoreCfg { max, level })
            }
        }

        deserializer.deserialize_any(SemaphoreVisitor)
    }
}

pub type Semaphores = HashMap<String, SemaphoreCfg>;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplicationCfg {
    #[serde(default = "HashMap::new")]
    pub semaphores: Semaphores,
    #[serde(default = "LoggingConfig::default")]
    pub logging: LoggingConfig,
}

impl ApplicationCfg {
    /// Checks for a file named `application.cfg` in the working directory. It is then used to
    /// create a new configuration. If the file can not be found a default configuration is created.
    pub fn init(path: &Path) -> Result<ApplicationCfg, Error> {
        match File::open(path) {
            Ok(mut file) => {
                let mut buffer = String::new();
                file.read_to_string(&mut buffer)
                    .map_err(Error::ReadConfigFile)?;
                let cfg = toml::from_str(&buffer).map_err(Error::DeserilizeToml)?;
                Ok(cfg)
            }
            Err(e) => {
                // Missing config file is fine and expected during local execution.
                if e.kind() == io::ErrorKind::NotFound {
                    eprintln!(
                        "{} not found => Using empty default configuration.",
                        path.to_string_lossy()
                    );
                    Ok(ApplicationCfg::default())
                } else {
                    eprintln!("{e}");
                    Err(Error::ReadConfigFile(e))
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
        let cfg = "[semaphores]\n\
                   A=1\n\
                   \n\
                ";
        let actual: ApplicationCfg = toml::from_str(cfg).unwrap();
        assert_eq!(actual.semaphores.get("A").unwrap().max, 1);
    }

    #[test]
    fn simple_and_verbose_configuration() {
        let simple = "
                     [semaphores]\n\
                     A=42\n\
                     \n\
                    ";
        let simple: ApplicationCfg = toml::from_str(simple).unwrap();

        let verbose = "
                      [semaphores]\n\
                      A = { max=42, level=0 }\n\
                      \n\
                    ";
        let verbose: ApplicationCfg = toml::from_str(verbose).unwrap();

        assert_eq!(simple, verbose);
    }

    /// Verify that the default configuration used in case of a missing file is identical to the
    /// configuration obtained from an empty toml file.
    #[test]
    fn default_configuration_equals_empty_configuration() {
        let empty: ApplicationCfg = toml::from_str("").unwrap();
        let default = ApplicationCfg::default();
        assert_eq!(empty, default);
    }

    #[test]
    fn parse_console_logging_config() {
        let cfg = "[logging.stderr]\n\
                    level = \"DEBUG\"\n\
                ";
        let actual: ApplicationCfg = toml::from_str(cfg).unwrap();
        assert_eq!(actual.logging.stderr.level, "DEBUG");
    }
}
