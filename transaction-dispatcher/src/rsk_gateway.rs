use crate::contracts::peg_manager::FakePegManagerContract;
use crate::contracts::peg_manager::notify_check_fork_complete::NotifyCheckForkCompleteInvoke;
use crate::contracts::signature_manager::{
    AddMemberNonceInvoke, AddMemberSignatureInvoke, SignatureManagerContract,
};

use crate::{
    config::TransactionConfig,
    contracts::peg_manager::{
        PegManagerContract, accept_peg_in_request::AcceptPegInRequestInvoke,
        get_temporary_peg_in_address::GetTemporaryPegInAddressCall,
        register_peg_in_request::RegisterPegInRequestInvoke,
        register_peg_out_request::RegisterPegOutRequestInvoke,
    },
    types::{
        AcceptPegInInput, AcceptPegInOutput, AddMemberNonceInput, AddMemberNonceOutput,
        AddMemberSignatureInput, AddMemberSignatureOutput, PegInAddressInput, PegInAddressOutput,
        RegisterPegInInput, RegisterPegInOutput, RegisterPegOutInput, RegisterPegOutOutput,
    },
};
use alloy_primitives::Address;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use common::types::ContractInfo;
use log::{error, info};
use std::collections::HashMap;
use thiserror::Error;

/// Must match the contract name in the config file
const PEG_MANAGER_CONTRACT_NAME: &str = "PegManager";
const FAKE_PEG_MANAGER_CONTRACT_NAME: &str = "FakePegManager";
const SIGNATURE_MANAGER_CONTRACT_NAME: &str = "SignatureManager";

pub trait RskContractsGatewayApi {
    fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> impl Future<Output = Result<PegInAddressOutput, DomainErrors>>;

    fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> impl Future<Output = Result<RegisterPegInOutput, DomainErrors>>;

    fn accept_peg_in_request(
        &self,
        input: AcceptPegInInput,
    ) -> impl Future<Output = Result<AcceptPegInOutput, DomainErrors>>;

    fn register_peg_out_request(
        &self,
        input: RegisterPegOutInput,
    ) -> impl Future<Output = Result<RegisterPegOutOutput, DomainErrors>>;

    fn add_member_nonce(
        &self,
        input: AddMemberNonceInput,
    ) -> impl Future<Output = Result<AddMemberNonceOutput, DomainErrors>>;

    fn add_member_signature(
        &self,
        input: AddMemberSignatureInput,
    ) -> impl Future<Output = Result<AddMemberSignatureOutput, DomainErrors>>;

    fn notify_check_fork_completion(
        &self,
        input: &str,
    ) -> impl Future<Output = Result<(), DomainErrors>>;
}

#[derive(Clone)]
pub struct RskContractsGateway<P: Provider> {
    contract_address: Address,
    get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall<PegManagerContract<P>>,
    register_peg_in_request_invoke: RegisterPegInRequestInvoke<PegManagerContract<P>>,
    accept_peg_in_request_invoke: AcceptPegInRequestInvoke<PegManagerContract<P>>,
    register_peg_out_request_invoke: RegisterPegOutRequestInvoke<PegManagerContract<P>>,
    add_member_nonce_invoke: AddMemberNonceInvoke<SignatureManagerContract<P>>,
    add_member_signature_invoke: AddMemberSignatureInvoke<SignatureManagerContract<P>>,
    notify_check_fork_completion_invoke: NotifyCheckForkCompleteInvoke<FakePegManagerContract<P>>,
}

impl<P: Provider + Clone> RskContractsGateway<P> {
    pub fn new(
        provider: P,
        managed_contracts: HashMap<String, ContractInfo>,
        tx_config: &TransactionConfig,
    ) -> Result<Self> {
        let contract_address = Self::load_contract(PEG_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let fake_contract_address =
            Self::load_contract(FAKE_PEG_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let signature_manager_address =
            Self::load_contract(SIGNATURE_MANAGER_CONTRACT_NAME, &managed_contracts)?;

        let peg_manager_contract = PegManagerContract::new(provider.clone(), contract_address);
        let fake_peg_manager_contract =
            FakePegManagerContract::new(provider.clone(), fake_contract_address);
        let signature_manager_contract =
            SignatureManagerContract::new(provider, signature_manager_address);

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
            register_peg_out_request_invoke: RegisterPegOutRequestInvoke::new(
                peg_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            notify_check_fork_completion_invoke: NotifyCheckForkCompleteInvoke::new(
                fake_peg_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            add_member_nonce_invoke: AddMemberNonceInvoke::new(
                signature_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            add_member_signature_invoke: AddMemberSignatureInvoke::new(
                signature_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
        })
    }

    fn load_contract(name: &str, contracts: &HashMap<String, ContractInfo>) -> Result<Address> {
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
    ) -> Result<PegInAddressOutput, DomainErrors> {
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
    ) -> Result<RegisterPegInOutput, DomainErrors> {
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
    ) -> Result<AcceptPegInOutput, DomainErrors> {
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

    async fn register_peg_out_request(
        &self,
        input: RegisterPegOutInput,
    ) -> Result<RegisterPegOutOutput, DomainErrors> {
        info!(
            "Interacting with PegManager#registerPegOutRequest @ {}",
            self.contract_address
        );

        self.register_peg_out_request_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on register_peg_out_request_invoke: {}", err);
                err
            })
    }

    async fn add_member_nonce(
        &self,
        input: AddMemberNonceInput,
    ) -> Result<AddMemberNonceOutput, DomainErrors> {
        info!(
            "Interacting with SignatureManager#addMemberNonce @ {}",
            self.contract_address
        );

        self.add_member_nonce_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on add_member_nonce_invoke: {}", err);
                err
            })
    }

    async fn add_member_signature(
        &self,
        input: AddMemberSignatureInput,
    ) -> Result<AddMemberSignatureOutput, DomainErrors> {
        info!(
            "Interacting with SignatureManager#addMemberSignature @ {}",
            self.contract_address
        );

        self.add_member_signature_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on add_member_signature_invoke: {}", err);
                err
            })
    }

    async fn notify_check_fork_completion(&self, input: &str) -> Result<(), DomainErrors> {
        info!(
            "Interacting with PegManager#notifyCheckForkCompletion @ {}",
            self.contract_address
        );

        self.notify_check_fork_completion_invoke
            .run(input)
            .await
            .map_err(|err| {
                error!("Error on notify_check_fork_completion_invoke: {}", err);
                err
            })
    }
}

#[derive(Debug, Error)]
pub enum DomainErrors {
    // mapped smart contract errors
    #[error("Pegin already requested: {0}")]
    PeginAlreadyRequested(String),
    #[error("Pegin already accepted: {0}")]
    PeginAlreadyAccepted(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid BTC Tx SPV Proof: {0}")]
    InvalidBtcTxSpvProof(String),
    #[error("Invalid compressed public key: {0}")]
    InvalidCompressedPubKey(String),
    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("Invalid value: {0}")]
    InvalidValue(String),
    #[error("Not Owner: {0}")]
    NotOwner(String),
    #[error("Not Enough Confirmations: {0}")]
    NotEnoughConfirmations(String),
    #[error("Pegout Request Amount Exceeds u64 Limit: {0}")]
    PegoutRequestAmountExceedsUint64Limit(String),
    #[error("Stream not found by denomination: {0}")]
    StreamNotFoundByDenomination(String),
    #[error("Packet out of bound: {0}")]
    PacketOutOfBound(String),

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
