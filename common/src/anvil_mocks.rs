#![cfg(feature = "anvil")]

use crate::types::BlockPow;
use primitive_types::{H256, U256};

pub fn get_anvil_block_pow() -> BlockPow {
    let value =
        U256::from_dec_str("46316835694926478169428394003475163141307993866256225615783033603")
            .expect("default_pow_header should not fail");
    BlockPow::from(H256::from_slice(&value.to_big_endian()))
}
