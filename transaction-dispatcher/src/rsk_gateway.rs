use std::collections::HashMap;
use std::error::Error;

use alloy_primitives::U256;
use alloy_provider::Provider;
use anyhow::{Result, anyhow};
use common::types::{Address, ContractInfo};
use log::{error, info};
#[cfg(test)]
use mockall::automock;
use thiserror::Error;

use crate::config::TransactionConfig;
use crate::contracts::committee_registry::{
    ApplyToStreamInvoke, CommitteeRegistryContract, DepositAggregatedKeysInvoke,
    DepositCommunicationDataInvoke, GetCommitteeCall, GetMemberCommunicationDataCall,
};
use crate::contracts::interactions::accept_pegin::AcceptPeginInvoke;
use crate::contracts::interactions::get_temporary_pegin_address::GetTemporaryPeginAddressCall;
use crate::contracts::interactions::notify_check_fork_complete::NotifyCheckForkCompleteInvoke;
use crate::contracts::interactions::register_operator_take::RegisterOperatorTakeInvoke;
use crate::contracts::interactions::register_pegout::RegisterPegoutInvoke;
use crate::contracts::interactions::request_pegin::RequestPeginInvoke;
use crate::contracts::interactions::request_pegout::TryPegoutInvoke;
use crate::contracts::interactions::trigger_operator_take::TriggerOperatorTakeInvoke;
use crate::contracts::member_registry::{GetMemberPublicKeysCall, MemberRegistryContract};
use crate::contracts::peg_manager::FakePegManagerContract;
use crate::contracts::pegin_manager::PeginManagerContract;
use crate::contracts::pegout_manager::PegoutManagerContract;
use crate::contracts::signature_manager::{
    AddMemberNonceInvoke, AddMemberSignatureInvoke, AddOperatorTakeTxHashInvoke,
    SignatureManagerContract,
};
use crate::contracts::stream_manager::StreamManagerContract;
use crate::types::{
    AcceptPeginInput, AcceptPeginOutput, AddMemberNonceInput, AddMemberNonceOutput,
    AddMemberSignatureInput, AddMemberSignatureOutput, AddOperatorTakeTxHashInput,
    AddOperatorTakeTxHashOutput, ApplyToStreamInput, ApplyToStreamOutput,
    DepositAggregatedKeyInput, DepositAggregatedKeyOutput, DepositCommunicationDataInput,
    DepositCommunicationDataOutput, GetCommitteeInput, GetCommitteeOutput,
    GetCommunicationDataInput, GetCommunicationDataOutput, GetMemberPublicKeysInput,
    GetMemberPublicKeysOutput, PeginAddressInput, PeginAddressOutput, RegisterOperatorTakeInput,
    RegisterOperatorTakeOutput, RegisterPegoutInput, RegisterPegoutOutput, RequestPeginInput,
    RequestPeginOutput, RequestPegoutInput, RequestPegoutOutput, TriggerOperatorTakeInput,
    TriggerOperatorTakeOutput,
};

/// Must match the contract name in the config file
const PEGIN_MANAGER_CONTRACT_NAME: &str = "PeginManager";
const PEGOUT_MANAGER_CONTRACT_NAME: &str = "PegoutManager";
const FAKE_PEG_MANAGER_CONTRACT_NAME: &str = "FakePegManager";
const SIGNATURE_MANAGER_CONTRACT_NAME: &str = "SignatureManager";
const COMMITTEE_REGISTRY_CONTRACT_NAME: &str = "CommitteeRegistry";
const MEMBER_REGISTRY_CONTRACT_NAME: &str = "MemberRegistry";
const STREAM_MANAGER_CONTRACT_NAME: &str = "StreamManager";

#[cfg_attr(test, automock)]
pub trait BalanceProvider {
    #[allow(async_fn_in_trait)]
    async fn get_balance(
        &self,
        addr: alloy_primitives::Address,
    ) -> Result<U256, Box<dyn Error + Send + Sync>>;
}

// implement BalanceProvider for any type that implements Provider
impl<T> BalanceProvider for T
where
    T: Provider,
{
    async fn get_balance(
        &self,
        addr: alloy_primitives::Address,
    ) -> Result<U256, Box<dyn Error + Send + Sync>> {
        self.get_balance(addr).await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
    }
}

pub trait RskContractsGatewayApi {
    fn my_address(&self) -> Address;

    fn get_balance(&self) -> impl Future<Output = Result<U256, DomainErrors>>;

    fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> impl Future<Output = Result<PeginAddressOutput, DomainErrors>>;

    fn request_pegin(
        &self,
        input: RequestPeginInput,
    ) -> impl Future<Output = Result<RequestPeginOutput, DomainErrors>>;

    fn accept_pegin(
        &self,
        input: AcceptPeginInput,
    ) -> impl Future<Output = Result<AcceptPeginOutput, DomainErrors>>;

    fn add_member_nonce(
        &self,
        input: AddMemberNonceInput,
    ) -> impl Future<Output = Result<AddMemberNonceOutput, DomainErrors>>;

    fn add_member_signature(
        &self,
        input: AddMemberSignatureInput,
    ) -> impl Future<Output = Result<AddMemberSignatureOutput, DomainErrors>>;

    fn add_operator_take_tx_hash(
        &self,
        input: AddOperatorTakeTxHashInput,
    ) -> impl Future<Output = Result<AddOperatorTakeTxHashOutput, DomainErrors>>;

    fn notify_check_fork_completion(
        &self,
        input: &str,
    ) -> impl Future<Output = Result<(), DomainErrors>>;

    fn request_pegout(
        &self,
        input: RequestPegoutInput,
    ) -> impl Future<Output = Result<RequestPegoutOutput, DomainErrors>>;

    fn register_pegout(
        &self,
        input: RegisterPegoutInput,
    ) -> impl Future<Output = Result<RegisterPegoutOutput, DomainErrors>>;

    fn register_operator_take(
        &self,
        input: RegisterOperatorTakeInput,
    ) -> impl Future<Output = Result<RegisterOperatorTakeOutput, DomainErrors>>;

    fn trigger_operator_take(
        &self,
        input: TriggerOperatorTakeInput,
    ) -> impl Future<Output = Result<TriggerOperatorTakeOutput, DomainErrors>>;

    fn get_member_public_keys(
        &self,
        input: GetMemberPublicKeysInput,
    ) -> impl Future<Output = Result<GetMemberPublicKeysOutput, DomainErrors>>;

    fn apply_to_stream(
        &self,
        input: ApplyToStreamInput,
    ) -> impl Future<Output = Result<ApplyToStreamOutput, DomainErrors>>;

    fn get_committee(
        &self,
        input: GetCommitteeInput,
    ) -> impl Future<Output = Result<GetCommitteeOutput, DomainErrors>>;

    fn get_committee_communication_data(
        &self,
        input: GetCommunicationDataInput,
    ) -> impl Future<Output = Result<GetCommunicationDataOutput, DomainErrors>>;

    fn deposit_communication_data(
        &self,
        input: DepositCommunicationDataInput,
    ) -> impl Future<Output = Result<DepositCommunicationDataOutput, DomainErrors>>;

    fn deposit_aggregated_key(
        &self,
        input: DepositAggregatedKeyInput,
    ) -> impl Future<Output = Result<DepositAggregatedKeyOutput, DomainErrors>>;
}

#[derive(Clone)]
pub struct RskContractsGateway<P: Provider + Clone> {
    provider: P,
    member_address: Address,
    get_temporary_pegin_address_call: GetTemporaryPeginAddressCall<PeginManagerContract<P>>,
    request_pegin_invoke: RequestPeginInvoke<PeginManagerContract<P>>,
    accept_pegin_invoke: AcceptPeginInvoke<PeginManagerContract<P>>,
    add_member_nonce_invoke: AddMemberNonceInvoke<SignatureManagerContract<P>>,
    add_member_signature_invoke: AddMemberSignatureInvoke<SignatureManagerContract<P>>,
    add_operator_take_tx_hash_invoke: AddOperatorTakeTxHashInvoke<SignatureManagerContract<P>>,
    notify_check_fork_completion_invoke: NotifyCheckForkCompleteInvoke<FakePegManagerContract<P>>,
    get_member_public_keys_call: GetMemberPublicKeysCall<MemberRegistryContract<P>>,
    get_member_communication_data_call:
        GetMemberCommunicationDataCall<CommitteeRegistryContract<P>>,
    apply_to_stream_invoke:
        ApplyToStreamInvoke<CommitteeRegistryContract<P>, StreamManagerContract<P>, P>,
    request_pegout_invoke: TryPegoutInvoke<PegoutManagerContract<P>>,
    register_pegout_invoke: RegisterPegoutInvoke<PegoutManagerContract<P>>,
    register_operator_take_invoke: RegisterOperatorTakeInvoke<PegoutManagerContract<P>>,
    trigger_operator_take_invoke: TriggerOperatorTakeInvoke<PegoutManagerContract<P>>,
    get_committee_call: GetCommitteeCall<CommitteeRegistryContract<P>>,
    deposit_communication_data_invoke: DepositCommunicationDataInvoke<CommitteeRegistryContract<P>>,
    deposit_aggregated_key_invoke: DepositAggregatedKeysInvoke<CommitteeRegistryContract<P>>,
}

impl<P: Provider + Clone> RskContractsGateway<P> {
    /// Create a new `RskContractsGateway` instance.
    ///
    /// # Errors
    ///
    /// Returns an error if any contract address is missing, invalid, or has no deployed code.
    #[allow(clippy::too_many_lines)]
    pub async fn new(
        // TODO make provider an Rc so we avoid more expensive cloning
        provider: P,
        managed_contracts: HashMap<String, ContractInfo>,
        tx_config: &TransactionConfig,
        member_address: Address,
    ) -> Result<Self> {
        let pegin_contract_address =
            Self::load_contract(PEGIN_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let pegout_contract_address =
            Self::load_contract(PEGOUT_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let fake_contract_address =
            Self::load_contract(FAKE_PEG_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let signature_manager_address =
            Self::load_contract(SIGNATURE_MANAGER_CONTRACT_NAME, &managed_contracts)?;
        let committee_registry_address =
            Self::load_contract(COMMITTEE_REGISTRY_CONTRACT_NAME, &managed_contracts)?;
        let member_registry_address =
            Self::load_contract(MEMBER_REGISTRY_CONTRACT_NAME, &managed_contracts)?;
        let stream_manager_address =
            Self::load_contract(STREAM_MANAGER_CONTRACT_NAME, &managed_contracts)?;

        // Validate that all contract addresses have deployed code
        let addresses_to_validate = vec![
            (PEGIN_MANAGER_CONTRACT_NAME, pegin_contract_address),
            (PEGOUT_MANAGER_CONTRACT_NAME, pegout_contract_address),
            // intentionally not validating fake peg manager contract
            (SIGNATURE_MANAGER_CONTRACT_NAME, signature_manager_address),
            (COMMITTEE_REGISTRY_CONTRACT_NAME, committee_registry_address),
            (MEMBER_REGISTRY_CONTRACT_NAME, member_registry_address),
            (STREAM_MANAGER_CONTRACT_NAME, stream_manager_address),
        ];

        for (contract_name, address) in &addresses_to_validate {
            let code = provider
                .get_code_at((*address).into())
                .await
                .map_err(|e| anyhow!("Failed to get code for contract {contract_name}: {e}"))?;

            if code.is_empty() {
                return Err(anyhow!(
                    "Contract {contract_name} at address {address} has no deployed code (0x)"
                ));
            }
        }

        // TODO make these contracts Rc so we avoid more expensive cloning

        let pegin_manager_contract =
            PeginManagerContract::new(provider.clone(), pegin_contract_address.into());
        let pegout_manager_contract =
            PegoutManagerContract::new(provider.clone(), pegout_contract_address.into());
        let fake_peg_manager_contract =
            FakePegManagerContract::new(provider.clone(), fake_contract_address.into());
        let signature_manager_contract =
            SignatureManagerContract::new(provider.clone(), signature_manager_address.into());
        let committee_registry_contract =
            CommitteeRegistryContract::new(provider.clone(), committee_registry_address.into());
        let member_registry_contract =
            MemberRegistryContract::new(provider.clone(), member_registry_address.into());
        let stream_manager_contract =
            StreamManagerContract::new(provider.clone(), stream_manager_address.into());

        Ok(RskContractsGateway {
            provider: provider.clone(),
            member_address,
            get_temporary_pegin_address_call: GetTemporaryPeginAddressCall::new(
                pegin_manager_contract.clone(),
            ),
            request_pegin_invoke: RequestPeginInvoke::new(
                pegin_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            accept_pegin_invoke: AcceptPeginInvoke::new(
                pegin_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            request_pegout_invoke: TryPegoutInvoke::new(
                pegout_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            notify_check_fork_completion_invoke: NotifyCheckForkCompleteInvoke::new(
                fake_peg_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            register_pegout_invoke: RegisterPegoutInvoke::new(
                pegout_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            register_operator_take_invoke: RegisterOperatorTakeInvoke::new(
                pegout_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            trigger_operator_take_invoke: TriggerOperatorTakeInvoke::new(
                pegout_manager_contract.clone(),
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
            add_operator_take_tx_hash_invoke: AddOperatorTakeTxHashInvoke::new(
                signature_manager_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            get_member_public_keys_call: GetMemberPublicKeysCall::new(
                member_registry_contract.clone(),
            ),
            get_member_communication_data_call: GetMemberCommunicationDataCall::new(
                committee_registry_contract.clone(),
            ),
            apply_to_stream_invoke: ApplyToStreamInvoke::new(
                committee_registry_contract.clone(),
                stream_manager_contract.clone(),
                tx_config.gas_bumps_t1,
                provider.clone(),
                alloy_primitives::Address::from(member_address),
            ),
            get_committee_call: GetCommitteeCall::new(committee_registry_contract.clone()),
            deposit_communication_data_invoke: DepositCommunicationDataInvoke::new(
                committee_registry_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
            deposit_aggregated_key_invoke: DepositAggregatedKeysInvoke::new(
                committee_registry_contract.clone(),
                tx_config.gas_bumps_t1,
            ),
        })
    }

    fn load_contract(name: &str, contracts: &HashMap<String, ContractInfo>) -> Result<Address> {
        contracts
            .get(name)
            .map(|info| info.address)
            .ok_or_else(|| anyhow!(format!("Address not found for contract: {name}")))
    }
}

impl<P: Provider + Clone> RskContractsGatewayApi for RskContractsGateway<P> {
    fn my_address(&self) -> Address {
        self.member_address
    }

    async fn get_balance(&self) -> Result<U256, DomainErrors> {
        self.provider
            .get_balance(self.member_address.into())
            .await
            .map_err(|e| DomainErrors::InternalServerError(format!("Failed to get balance: {e}")))
    }

    async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors> {
        info!("Interacting with PeginManager#getRequestPeginData",);

        self.get_temporary_pegin_address_call.run(input).await.map_err(|err| {
            error!("Error on get_temporary_pegin_address_call: {err}");
            err
        })
    }

    async fn request_pegin(
        &self,
        input: RequestPeginInput,
    ) -> Result<RequestPeginOutput, DomainErrors> {
        info!("Interacting with PeginManager#requestPegin",);

        self.request_pegin_invoke.run(input).await.map_err(|err| {
            error!("Error on request_pegin_invoke: {err}");
            err
        })
    }

    async fn accept_pegin(
        &self,
        input: AcceptPeginInput,
    ) -> Result<AcceptPeginOutput, DomainErrors> {
        info!("Interacting with PeginManager#acceptPegin",);

        self.accept_pegin_invoke.run(input).await.map_err(|err| {
            error!("Error on accept_pegin_invoke: {err}");
            err
        })
    }

    async fn add_member_nonce(
        &self,
        input: AddMemberNonceInput,
    ) -> Result<AddMemberNonceOutput, DomainErrors> {
        info!("Interacting with SignatureManager#addMemberNonce",);

        self.add_member_nonce_invoke.run(input).await.map_err(|err| {
            error!("Error on add_member_nonce_invoke: {err}");
            err
        })
    }

    async fn add_member_signature(
        &self,
        input: AddMemberSignatureInput,
    ) -> Result<AddMemberSignatureOutput, DomainErrors> {
        info!("Interacting with SignatureManager#addMemberSignature");

        self.add_member_signature_invoke.run(input).await.map_err(|err| {
            error!("Error on add_member_signature_invoke: {err}");
            err
        })
    }

    async fn add_operator_take_tx_hash(
        &self,
        input: AddOperatorTakeTxHashInput,
    ) -> Result<AddMemberNonceOutput, DomainErrors> {
        info!("Interacting with SignatureManager#addOperatorTakeTxHash",);

        self.add_operator_take_tx_hash_invoke.run(input).await.map_err(|err| {
            error!("Error on add_operator_take_tx_hash_invoke: {err}");
            err
        })
    }

    async fn notify_check_fork_completion(&self, input: &str) -> Result<(), DomainErrors> {
        info!("Interacting with FakePegManager#notifyCheckForkCompletion",);

        self.notify_check_fork_completion_invoke.run(input).await.map_err(|err| {
            error!("Error on notify_check_fork_completion_invoke: {err}");
            err
        })
    }

    async fn request_pegout(
        &self,
        input: RequestPegoutInput,
    ) -> Result<RequestPegoutOutput, DomainErrors> {
        info!("Interacting with PegoutManager#tryPegoutRequest",);

        self.request_pegout_invoke.run(input).await.map_err(|err| {
            error!("Error on try_pegout_invoke: {err}");
            err
        })
    }

    async fn register_pegout(
        &self,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput, DomainErrors> {
        info!("Interacting with PegoutManager#registerUserTake");

        self.register_pegout_invoke.run(input).await.map_err(|err| {
            error!("Error on register_pegout_invoke: {err}");
            err
        })
    }

    async fn register_operator_take(
        &self,
        input: RegisterOperatorTakeInput,
    ) -> Result<RegisterOperatorTakeOutput, DomainErrors> {
        info!("Interacting with PegoutManager#registerOperatorTake");

        self.register_operator_take_invoke.run(input).await.map_err(|err| {
            error!("Error on register_operator_take_invoke: {err}");
            err
        })
    }

    async fn trigger_operator_take(
        &self,
        input: TriggerOperatorTakeInput,
    ) -> Result<TriggerOperatorTakeOutput, DomainErrors> {
        info!("Interacting with PegoutManager#triggerOperatorTake");

        self.trigger_operator_take_invoke.run(input).await.map_err(|err| {
            error!("Error on trigger_operator_take_invoke: {err}");
            err
        })
    }

    async fn get_member_public_keys(
        &self,
        input: GetMemberPublicKeysInput,
    ) -> Result<GetMemberPublicKeysOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#getMemberPublicKeys",);

        self.get_member_public_keys_call.run(input).await.map_err(|err| {
            error!("Error on get_member_public_keys_call: {err}");
            err
        })
    }

    async fn apply_to_stream(
        &self,
        input: ApplyToStreamInput,
    ) -> Result<ApplyToStreamOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#applyToStream",);

        self.apply_to_stream_invoke.run(input).await.map_err(|err| {
            error!("Error on apply_to_stream_invoke: {err}");
            err
        })
    }

    async fn get_committee(
        &self,
        input: GetCommitteeInput,
    ) -> Result<GetCommitteeOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#getCommittee");

        self.get_committee_call.run(input).await.map_err(|err| {
            error!("Error on get_committee_call: {err}");
            err
        })
    }

    async fn get_committee_communication_data(
        &self,
        input: GetCommunicationDataInput,
    ) -> Result<GetCommunicationDataOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#getMemberCommunicationData",);

        self.get_member_communication_data_call.run(input).await.map_err(|err| {
            error!("Error on get_member_communication_data_call: {err}");
            err
        })
    }

    async fn deposit_communication_data(
        &self,
        input: DepositCommunicationDataInput,
    ) -> Result<DepositCommunicationDataOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#depositCommunicationData");

        self.deposit_communication_data_invoke.run(input).await.map_err(|err| {
            error!("Error on deposit_communication_data_invoke: {err}");
            err
        })
    }

    async fn deposit_aggregated_key(
        &self,
        input: DepositAggregatedKeyInput,
    ) -> Result<DepositAggregatedKeyOutput, DomainErrors> {
        info!("Interacting with CommitteeRegistry#depositAggregatedKeys",);

        self.deposit_aggregated_key_invoke.run(input).await.map_err(|err| {
            error!("Error on deposit_aggregated_key_invoke: {err}");
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
    #[error("Pegout Request Amount Exceeds u64 Limit: {0}")]
    PegoutRequestAmountExceedsUint64Limit(String),
    #[error("Stream not found by denomination: {0}")]
    StreamNotFoundByDenomination(String),
    #[error("Packet out of bound: {0}")]
    PacketOutOfBound(String),
    #[error("Invalid role: {0}")]
    InvalidRole(String),
    #[error("Error interacting with Committee: {0}")]
    CommitteeError(String),
    #[error("Member already deposited communication data: {0}")]
    MemberAlreadyDepositedCommunicationData(String),
    #[error("Member already registered for stream: {0}")]
    MemberAlreadyRegisteredForStream(String),
    #[error("Member info already deposited: {0}")]
    MemberInfoAlreadyDeposited(String),
    #[error("Error interacting with MemberRegistry: {0}")]
    MemberRegistryError(String),
    #[error("Error collecting signatures: {0}")]
    SignaturesError(String),
    #[error("Missing confirmations on native bridge: {0}")]
    MissingConfirmationsOnNativeBridge(String),
    #[error("Invalid slot state: expected {expected}, actual {actual}")]
    InvalidSlotState { expected: u8, actual: u8 },

    // unhandled smart contract errors
    #[error("Unhandled Contract Error: {0}")]
    UnhandledContractError(String),

    // not smart contract errors
    #[error("No Revert Error: {0}")]
    NoRevertError(String),

    // unexpected errors
    #[error("Unknown Contract Error: {0}")]
    UnknownContractError(String),

    #[error("Internal non-contract error: {0}")]
    InternalServerError(String),
}
