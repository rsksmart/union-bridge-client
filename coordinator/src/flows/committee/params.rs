use anyhow::{Context, Result};

const DEFAULT_SLOTS_PER_PACKAGE: u32 = 100;
const DEFAULT_COMMITTEE_MEMBER_COUNT: u64 = 4;
const DEFAULT_COMMITTEE_PROVER_COUNT: u64 = 2;

// TODO: Move these behind shared protocol parameter resolution once `common` is split into
// lighter crates, so small consumers can share these values without pulling in the current broad
// dependency graph. Prefer authoritative runtime sources where available: derive committee member
// count from registered committee state, and derive slot/package limits from contracts only if
// contracts expose or enforce them. Keep shared config/env values for parameters not exposed by
// contracts yet, such as prover count, and for local/dev testing.

pub(super) fn committee_member_count() -> Result<u64> {
    env_u64_or_default("COMMITTEE_MEMBER_COUNT", DEFAULT_COMMITTEE_MEMBER_COUNT)
}

pub(super) fn prover_count() -> Result<u64> {
    env_u64_or_default("COMMITTEE_PROVER_COUNT", DEFAULT_COMMITTEE_PROVER_COUNT)
}

pub(super) fn slots_per_package() -> Result<u64> {
    Ok(u64::from(bitvmx_slots_per_package()?))
}

pub(super) fn bitvmx_slots_per_package() -> Result<u32> {
    match std::env::var("SLOTS_PER_PACKAGE") {
        Ok(value) => value.parse::<u32>().context("SLOTS_PER_PACKAGE must be a valid u32"),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_SLOTS_PER_PACKAGE),
        Err(err) => Err(err).context("failed to read SLOTS_PER_PACKAGE"),
    }
}

fn env_u64_or_default(env_var: &str, default: u64) -> Result<u64> {
    match std::env::var(env_var) {
        Ok(value) => value.parse::<u64>().with_context(|| format!("{env_var} must be a valid u64")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed to read {env_var}")),
    }
}
