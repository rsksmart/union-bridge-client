use anyhow::{Context, Result};
use protocol_params::slots_per_package;

pub(super) fn bitvmx_slots_per_package() -> Result<u32> {
    u32::try_from(slots_per_package()?).context("SLOTS_PER_PACKAGE must fit in u32 for BitVMX")
}
