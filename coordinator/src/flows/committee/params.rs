use anyhow::{Context, Result};

// TODO: Move these behind shared protocol parameter resolution once `common` is split into
// lighter crates, so small consumers can share these values without pulling in the current broad
// dependency graph. Prefer authoritative runtime sources where available: derive committee member
// count from registered committee state, and derive slot/package limits from contracts only if
// contracts expose or enforce them. Keep shared config/env values for parameters not exposed by
// contracts yet, such as prover count, and for local/dev testing.

pub(super) fn committee_member_count() -> Result<u64> {
    required_env_u64("COMMITTEE_MEMBER_COUNT")
}

pub(super) fn prover_count() -> Result<u64> {
    required_env_u64("COMMITTEE_PROVER_COUNT")
}

pub(super) fn slots_per_package() -> Result<u64> {
    Ok(u64::from(bitvmx_slots_per_package()?))
}

pub(super) fn bitvmx_slots_per_package() -> Result<u32> {
    std::env::var("SLOTS_PER_PACKAGE")
        .context("SLOTS_PER_PACKAGE environment variable is required")?
        .parse::<u32>()
        .context("SLOTS_PER_PACKAGE must be a valid u32")
}

fn required_env_u64(env_var: &str) -> Result<u64> {
    std::env::var(env_var)
        .with_context(|| format!("{env_var} environment variable is required"))?
        .parse::<u64>()
        .with_context(|| format!("{env_var} must be a valid u64"))
}
