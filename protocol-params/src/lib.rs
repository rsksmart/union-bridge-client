//! Shared protocol sizing defaults.
//!
//! TODO: This is a temporary workaround until we gather them from contracts.
//! Keep env/config values only for parameters not exposed by contracts yet, such as prover count,
//! and for local/dev
//! testing.

use anyhow::{Context, Result};

/// Returns the committee member count from the temporary env override or the default.
///
/// # Errors
///
/// Returns an error when `COMMITTEE_MEMBER_COUNT` is present but is not a valid `u64`, or when the
/// environment variable cannot be read.
pub fn committee_member_count() -> Result<u64> {
    env_u64_or_default("COMMITTEE_MEMBER_COUNT", 4)
}

/// Returns the committee prover count from the temporary env override or the default.
///
/// # Errors
///
/// Returns an error when `COMMITTEE_PROVER_COUNT` is present but is not a valid `u64`, or when the
/// environment variable cannot be read.
pub fn prover_count() -> Result<u64> {
    env_u64_or_default("COMMITTEE_PROVER_COUNT", 2)
}

/// Returns the slots-per-package value from the temporary env override or the default.
///
/// # Errors
///
/// Returns an error when `SLOTS_PER_PACKAGE` is present but is not a valid `u64`, or when the
/// environment variable cannot be read.
pub fn slots_per_package() -> Result<u64> {
    env_u64_or_default("SLOTS_PER_PACKAGE", 100)
}

fn env_u64_or_default(env_var: &str, default: u64) -> Result<u64> {
    match std::env::var(env_var) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(default)
            } else {
                trimmed.parse::<u64>().with_context(|| format!("{env_var} must be a valid u64"))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed to read {env_var}")),
    }
}
