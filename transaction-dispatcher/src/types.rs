use alloy_primitives::Address;
use alloy_provider::RootProvider;
use anyhow::{Context, Result};
use common::types::ContractInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Debug)]
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Debug)]
pub struct PeginAddressOutput {
    pub address: String,
}

pub trait BaseContract {
    fn new(provider: &RootProvider, contracts: HashMap<String, ContractInfo>) -> Result<Self>
    where
        Self: Sized,
    {
        let contract_info = contracts.get(&Self::contract_name()).context(format!(
            "Address not found for contract: {}",
            Self::contract_name()
        ))?;

        let address = contract_info.address.parse().context(format!(
            "Could not parse contract address for: {}",
            Self::contract_name()
        ))?;

        Self::init(&provider, address)
    }

    fn init(provider: &RootProvider, address: Address) -> Result<Self>
    where
        Self: Sized;

    /// Must match the contract name in the ABI file
    fn contract_name() -> String;
}
