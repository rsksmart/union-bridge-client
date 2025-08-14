use crate::contracts::common::send_tx_with_gas_bump;
use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;
use alloy_primitives::U256;
use alloy_provider::Provider;
use log::info;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{self, Committee};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    CommitteeRegistryErrors, CommitteeRegistryInstance, StreamDenomination,
};

#[cfg(test)]
use mockall::automock;

pub(crate) use crate::contracts::interactions::apply_to_stream::ApplyToStreamInvoke;
pub(crate) use crate::contracts::interactions::deposit_communication_data::DepositCommunicationDataInvoke;
pub(crate) use crate::contracts::interactions::get_committee::GetCommitteeCall;
pub(crate) use crate::contracts::interactions::get_member_communication_data::GetMemberCommunicationDataCall;
pub(crate) use crate::contracts::interactions::get_member_public_keys::GetMemberPublicKeysCall;

#[cfg_attr(test, automock)]
pub trait CommitteeRegistryContractApi {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<alloy_primitives::FixedBytes<32>>>;

    async fn call_get_member_communication_data(
        &self,
        stream_id: u64,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<CommitteeRegistry::CommunicationData>>;

    async fn invoke_apply_to_stream(
        &self,
        stream: u8,
        role: u8,
        public_keys: Vec<CommitteeRegistry::PublicKeyRegistration>,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt>;

    async fn call_get_minimum_deposit(
        &self,
        stream: StreamDenomination,
    ) -> alloy_contract::Result<U256>;

    async fn call_get_committee(&self, committee_id: U256) -> alloy_contract::Result<Committee>;

    async fn invoke_deposit_communication_data(
        &self,
        stream_id: u64,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt>;
}

#[derive(Clone)]
pub struct CommitteeRegistryContract<P: Provider> {
    contract_instance: CommitteeRegistryInstance<P>,
}

impl<P: Provider> CommitteeRegistryContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!(
            "Connecting to CommitteeRegistry Contract @ {}",
            contract_address
        );
        let contract_instance = CommitteeRegistry::new(contract_address, provider);
        CommitteeRegistryContract { contract_instance }
    }
}

impl<P: Provider> CommitteeRegistryContractApi for CommitteeRegistryContract<P> {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<alloy_primitives::FixedBytes<32>>> {
        self.contract_instance
            .getMemberPublicKeys(member_address)
            .call()
            .await
    }

    async fn call_get_member_communication_data(
        &self,
        stream_id: u64,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<CommitteeRegistry::CommunicationData>> {
        self.contract_instance
            .getMemberCommunicationData(stream_id, member_address)
            .call()
            .await
    }

    async fn invoke_apply_to_stream(
        &self,
        stream: u8,
        role: u8,
        public_keys: Vec<CommitteeRegistry::PublicKeyRegistration>,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt> {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .applyToStream(stream, role, public_keys.clone())
                    .value(value)
            },
            gas_bumps,
        )
        .await
    }

    async fn call_get_minimum_deposit(
        &self,
        _stream: StreamDenomination,
    ) -> alloy_contract::Result<U256> {
        // TODO(207): Resolve to new getMinimumDeposit method from alpha3
        // self.contract_instance.getMissingCommunicationDataCount
        //     .getMinimumDeposit(u8::from(stream))
        //     .call()
        //     .await
        
        // Temporary hardcoded minimum deposit (0.025 RBTC = 25000000000000000 wei)
        Ok(U256::from(25000000000000000u64))
    }

    async fn call_get_committee(&self, committee_id: U256) -> alloy_contract::Result<Committee> {
        self.contract_instance
            .getCommittee(committee_id)
            .call()
            .await
    }

    async fn invoke_deposit_communication_data(
        &self,
        stream_id: u64,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt> {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .depositCommunicationData(stream_id, communication_data.clone())
            },
            gas_bumps,
        )
        .await
    }
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<CommitteeRegistryErrors>();
    decoded_err.map(|e| DomainErrors::CommitteeError(format!("{:?}", e)))
}
