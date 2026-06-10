use config;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Error while trying to build configuration")]
    ConfigFileError(#[from] config::ConfigError),
    #[error("Invalid environment name")]
    ConfigEnvError(String),
}
