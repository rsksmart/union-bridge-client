// default operator IDs for local deployments (overridable via NUM_OPERATORS env var)
use anyhow::{Context, Result};

pub const DEFAULT_NUM_OPERATORS: u8 = 4;
pub const MAX_OPERATORS: u8 = 10;

pub fn operator_ids() -> Vec<u8> {
    let count = std::env::var("NUM_OPERATORS")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(DEFAULT_NUM_OPERATORS)
        .min(MAX_OPERATORS);
    (1..=count).collect()
}

// TODO: Move these behind shared protocol parameter resolution once `common` is split into
// lighter crates, so small consumers can share these values without pulling in the current broad
// dependency graph. Prefer authoritative runtime sources where available: derive committee member
// count from registered committee state, and derive slot/package limits from contracts only if
// contracts expose or enforce them. Keep shared config/env values for parameters not exposed by
// contracts yet, such as prover count, and for local/dev testing.
pub fn committee_member_count() -> Result<u64> {
    required_env_u64("COMMITTEE_MEMBER_COUNT")
}

pub fn prover_count() -> Result<u64> {
    required_env_u64("COMMITTEE_PROVER_COUNT")
}

pub fn slots_per_package() -> Result<u64> {
    Ok(u64::from(
        std::env::var("SLOTS_PER_PACKAGE")
            .context("SLOTS_PER_PACKAGE environment variable is required")?
            .parse::<u32>()
            .context("SLOTS_PER_PACKAGE must be a valid u32")?,
    ))
}

fn required_env_u64(env_var: &str) -> Result<u64> {
    std::env::var(env_var)
        .with_context(|| format!("{env_var} environment variable is required"))?
        .parse::<u64>()
        .with_context(|| format!("{env_var} must be a valid u64"))
}

// local anvil default address
pub const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

pub const UNION_BRIDGE_DIR: &str = ".union_bridge";
