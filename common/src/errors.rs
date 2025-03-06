use config;
use hex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlockHashError {
    #[error("Invalid hex string: {0}")]
    InvalidHex(#[from] hex::FromHexError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Error while trying to build configuration")]
    ConfigFileError(#[from] config::ConfigError),
}
