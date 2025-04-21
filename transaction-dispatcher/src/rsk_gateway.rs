use crate::config::TransactionConfig;
use crate::contracts::peg_manager::PegManagerContract;
use crate::contracts::peg_manager::accept_peg_in_request::AcceptPegInRequestInvoke;
use crate::contracts::peg_manager::get_temporary_peg_in_address::GetTemporaryPegInAddressCall;
use crate::contracts::peg_manager::register_peg_in_request::RegisterPegInRequestInvoke;
use crate::types::{
    AcceptPegInInput, AcceptPegInOutput, PegInAddressInput, PegInAddressOutput, RegisterPegInInput,
    RegisterPegInOutput,
};
use alloy_primitives::Address;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use common::types::ContractInfo;
use log::{error, info};
use std::collections::HashMap;
use thiserror::Error;

/// Must  match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &'static str = "PegManager";

pub(crate) trait RskContractsGatewayApi {
    async fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors>;

    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<RegisterPegInOutput, PegManagerErrors>;

    async fn accept_peg_in_request(
        &self,
        input: AcceptPegInInput,
    ) -> Result<AcceptPegInOutput, PegManagerErrors>;
}

pub struct RskContractsGateway<P: Provider> {
    contract_address: Address,
    get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall<PegManagerContract<P>>,
    register_peg_in_request_invoke: RegisterPegInRequestInvoke<PegManagerContract<P>>,
    accept_peg_in_request_invoke: AcceptPegInRequestInvoke<PegManagerContract<P>>,
}

impl<P: Provider + Clone> RskContractsGateway<P> {
    pub fn new(
        provider: P,
        managed_contracts: HashMap<String, ContractInfo>,
        tx_config: &TransactionConfig,
    ) -> Result<Self> {
        let contract_address = Self::load_contract(PEG_MANAGER_CONTRACT_NAME, managed_contracts)?;

        let peg_manager_contract = PegManagerContract::new(provider, contract_address);

        Ok(RskContractsGateway {
            contract_address,
            get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall::new(
                peg_manager_contract.clone(),
            ),
            register_peg_in_request_invoke: RegisterPegInRequestInvoke::new(
                peg_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            accept_peg_in_request_invoke: AcceptPegInRequestInvoke::new(
                peg_manager_contract.clone(),
                tx_config.gas_bumps_t1,
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

        self.get_temporary_peg_in_address_call
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on get_temporary_peg_in_address_call: {}", err);
                err
            })
    }

    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<RegisterPegInOutput, PegManagerErrors> {
        info!(
            "Interacting with PegManager#registerPegInRequest @ {}",
            self.contract_address
        );

        self.register_peg_in_request_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on register_peg_in_request_invoke: {}", err);
                err
            })
    }

    async fn accept_peg_in_request(
        &self,
        input: AcceptPegInInput,
    ) -> Result<AcceptPegInOutput, PegManagerErrors> {
        info!(
            "Interacting with PegManager#acceptPegInRequest @ {}",
            self.contract_address
        );

        self.accept_peg_in_request_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on accept_peg_in_request_invoke: {}", err);
                err
            })
    }
}

#[derive(Debug, Error)]
pub enum PegManagerErrors {
    // mapped smart contract errors
    #[error("Already registered PegIn: {0}")]
    AlreadyRegisteredPegIn(String),
    #[error("Already registered Accept PegIn: {0}")]
    AlreadyRegisteredAcceptPegIn(String),
    #[error("Already registered PegIn Request: {0}")]
    AlreadyRegisteredPegInRequest(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid BTC Tx SPV Proof: {0}")]
    InvalidBtcTxSpvProof(String),
    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("Invalid value: {0}")]
    InvalidValue(String),
    #[error("Not Owner: {0}")]
    NotOwner(String),
    #[error("Not Enough Confirmations: {0}")]
    NotEnoughConfirmations(String),
    #[error("Packet out of Bound: {0}")]
    PacketOutOfBound(String),
    #[error("Stream not found by denomination: {0}")]
    StreamNotFoundByDenomination(String),
    #[error("Unregistered Request: {0}")]
    UnregisteredRequest(String),

    // unhandled smart contract errors
    #[error("Unhandled Contract Error: {0}")]
    UnhandledContractError(String),

    // not smart contract errors
    #[error("No Revert Error: {0}")]
    NoRevertError(String),

    // unexpected errors
    #[error("Unknown Contract Error: {0}")]
    UnknownContractError(String),
}
