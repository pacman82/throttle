use crate::logging::LoggingConfig;
use serde::{de, Deserialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::Path,
    time::Duration,
};

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
pub struct Semaphore {
    pub count: i64,
}

impl<'de> de::Deserialize<'de> for Semaphore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct SemaphoreVisitor;

        /// Repetition of Semaphore, but with derived `Deserialize` Trait.
        #[derive(Deserialize)]
        pub struct Verbose {
            pub count: i64,
        }

        impl<'de> de::Visitor<'de> for SemaphoreVisitor {
            type Value = Semaphore;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "a semaphore count like 42 or a verbose semaphore configuration like \
                    { count = 42 }",
                )
            }

            fn visit_i64<E>(self, i: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Semaphore { count: i })
            }

            fn visit_map<V>(self, map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mvd = de::value::MapAccessDeserializer::new(map);
                Verbose::deserialize(mvd).map(|Verbose { count }| Semaphore { count })
            }
        }

        deserializer.deserialize_any(SemaphoreVisitor)
    }
}

pub type Semaphores = HashMap<String, Semaphore>;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ApplicationCfg {
    #[serde(
        with = "humantime_serde",
        default = "ApplicationCfg::litter_collection_interval_default"
    )]
    pub litter_collection_interval: Duration,
    #[serde(default = "HashMap::new")]
    pub semaphores: Semaphores,
    #[serde(default = "LoggingConfig::default")]
    pub logging: LoggingConfig,
}

impl Default for ApplicationCfg {
    fn default() -> ApplicationCfg {
        ApplicationCfg {
            litter_collection_interval: Duration::from_secs(300), // 5min
            semaphores: HashMap::new(),
            logging: LoggingConfig::default(),
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
                    let err = e;
                    eprintln!("{}", err);
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
        assert_eq!(
            actual.litter_collection_interval,
            Duration::from_millis(100)
        );
        assert_eq!(actual.semaphores.get("A").unwrap().count, 1);
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
                      A = { count=42 }\n\
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

    /// Verify format of configuring gelf parser
    #[test]
    fn parse_gelf_logging_config() {
        let cfg = "[logging.gelf]\n\
                    name = \"MyThrottleServer.net\"\n\
                    host = \"my_graylog_instance.cloud\"\n\
                    port = 12201\n\
                    level = \"DEBUG\"\n\
                ";
        let actual: ApplicationCfg = toml::from_str(cfg).unwrap();
        assert!(actual.logging.gelf.is_some());
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
