use crate::contracts::peg_manager::get_temporary_peg_in_address::GetTemporaryPegInAddressCall;
use crate::contracts::peg_manager::register_peg_in_request::RegisterPegInRequestInvoke;
use crate::contracts::peg_manager::PegManagerContract;
use crate::types::{
    PegInAddressInput, PegInAddressOutput, RegisterPegInInput, RegisterPegInOutput,
};
use alloy_primitives::Address;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use common::{config::Config, types::ContractInfo};
use log::info;
use std::collections::HashMap;
use thiserror::Error;

/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub(crate) trait RskContractsGatewayApi {
    #[allow(async_fn_in_trait)]
    async fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors>;

    #[allow(async_fn_in_trait)]
    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<RegisterPegInOutput, PegManagerErrors>;
}

pub struct RskContractsGateway<P: Provider> {
    contract_address: Address,
    get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall<PegManagerContract<P>>,
    register_peg_in_request_invoke: RegisterPegInRequestInvoke<PegManagerContract<P>>,
}

impl<P: Provider + Clone> RskContractsGateway<P> {
    pub fn new(provider: P, config: &Config) -> Result<Self> {
        let contract_address =
            Self::load_contract(PEG_MANAGER_CONTRACT_NAME, config.load_managed_contracts(true))?;

        let peg_manager_contract = PegManagerContract::new(provider, contract_address);

        Ok(RskContractsGateway {
            contract_address,
            get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall::new(
                peg_manager_contract.clone(),
            ),
            register_peg_in_request_invoke: RegisterPegInRequestInvoke::new(
                peg_manager_contract.clone(),
            ),
        })
    }

    fn load_contract(name: &str, contracts: HashMap<String, ContractInfo>) -> Result<Address> {
        contracts
            .get(name)
            .context(format!("Address not found for contract: {}", name))?
            .address
            .to_string()
            .parse::<Address>()
            .context("Parsing to Address failed")
    }
}

impl<P: Provider> RskContractsGatewayApi for RskContractsGateway<P> {
    async fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors> {
        info!(
            "Interacting with PegManager#getTemporaryPegInAddress @ {}",
            self.contract_address
        );

        self.get_temporary_peg_in_address_call.run(input).await
    }

    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<RegisterPegInOutput, PegManagerErrors> {
        info!(
            "Interacting with PegManager#registerPegInRequest @ {}",
            self.contract_address
        );

        self.register_peg_in_request_invoke.run(input).await
    }
}

// TODO(iago) add parameters to the error to avoid the extra error! log
#[derive(Debug, Error)]
pub enum PegManagerErrors {
    #[error("No Revert Error: {0}")]
    NoRevertError(String),
    #[error("Unknown Contract Error: {0}")]
    UnknownContractError(String),
    #[error("Unhandled Contract Error")]
    UnhandledContractError,
    #[error("Stream not found by denomination")]
    StreamNotFoundByDenomination,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid address")]
    InvalidAddress,
    #[error("Already registered PegIn")]
    AlreadyRegisteredPegIn,
    #[error("Invalid data in PegIn transaction")]
    InvalidPegInRequestData,
    #[error("Invalid value")]
    InvalidValue,
}
