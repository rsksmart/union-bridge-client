use alloy_primitives::hex::FromHexError;
use alloy_primitives::ruint::ParseError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}
