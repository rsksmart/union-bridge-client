use config;
use hex::FromHexError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlockHashError {
    #[error("Invalid hex string: {0}")]
    InvalidHex(#[from] FromHexError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Error while trying to build configuration")]
    ConfigFileError(#[from] config::ConfigError),
}
