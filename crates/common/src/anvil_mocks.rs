#![cfg(feature = "anvil")]

use std::str::FromStr;

use primitive_types::{H256, U256};

use crate::types::BlockPow;

/// # Panics
///
/// Panics if the default pow header cannot be parsed.
#[must_use]
pub fn get_anvil_block_pow() -> BlockPow {
    // ~= difficulty x 20
    let value =
        U256::from_str("0x00000000000000000001705df3f37d4895e9a579efd7a96e045cf020f1f510ef")
            .expect("default_pow_header should not fail");
    BlockPow::from(H256::from_slice(&value.to_big_endian()))
}
