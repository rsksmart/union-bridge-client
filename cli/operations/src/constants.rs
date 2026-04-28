// default operator IDs for local deployments (overridable via NUM_OPERATORS env var)
use anyhow::{Context, Result};

pub const DEFAULT_NUM_OPERATORS: u8 = 4;
pub const MAX_OPERATORS: u8 = 10;
const DEFAULT_SLOTS_PER_PACKAGE: u64 = 100;
const DEFAULT_COMMITTEE_MEMBER_COUNT: u64 = 4;
const DEFAULT_COMMITTEE_PROVER_COUNT: u64 = 2;

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
    env_u64_or_default("COMMITTEE_MEMBER_COUNT", DEFAULT_COMMITTEE_MEMBER_COUNT)
}

pub fn prover_count() -> Result<u64> {
    env_u64_or_default("COMMITTEE_PROVER_COUNT", DEFAULT_COMMITTEE_PROVER_COUNT)
}

pub fn slots_per_package() -> Result<u64> {
    env_u64_or_default("SLOTS_PER_PACKAGE", DEFAULT_SLOTS_PER_PACKAGE)
}

fn env_u64_or_default(env_var: &str, default: u64) -> Result<u64> {
    match std::env::var(env_var) {
        Ok(value) => value.parse::<u64>().with_context(|| format!("{env_var} must be a valid u64")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed to read {env_var}")),
    }
}

// local anvil default address
pub const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

pub const UNION_BRIDGE_DIR: &str = ".union_bridge";
