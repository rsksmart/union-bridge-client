use crate::contracts::common::send_tx_with_gas_bump;
use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;
use alloy_primitives::U256;
use alloy_provider::Provider;
use log::info;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{self, Committee, Role};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    CommitteeRegistryErrors, CommitteeRegistryInstance, MemberKeys, MemberRegistrationKeys,
    StreamDenomination, UTXO,
};

pub(crate) use crate::contracts::interactions::apply_to_stream::ApplyToStreamInvoke;
pub(crate) use crate::contracts::interactions::deposit_aggregated_key::DepositAggregatedKeysInvoke;
pub(crate) use crate::contracts::interactions::deposit_communication_data::DepositCommunicationDataInvoke;
pub(crate) use crate::contracts::interactions::get_committee::GetCommitteeCall;
pub(crate) use crate::contracts::interactions::get_member_communication_data::GetMemberCommunicationDataCall;
pub(crate) use crate::contracts::interactions::get_member_funding_utxo::GetMemberFundingUtxoCall;
pub(crate) use crate::contracts::interactions::get_member_public_keys::GetMemberPublicKeysCall;
use common::types::{CommitteeId, StreamId};
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait CommitteeRegistryContractApi {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<MemberKeys>;

    async fn call_get_member_communication_data(
        &self,
        committee_id: CommitteeId,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<CommitteeRegistry::CommunicationData>>;

    async fn call_get_member_funding_utxo(
        &self,
        committee_id: StreamId,
        member_address: Address,
    ) -> alloy_contract::Result<UTXO>;

    async fn invoke_apply_to_stream(
        &self,
        denomination: StreamDenomination,
        role: Role,
        public_keys: MemberRegistrationKeys,
        funding_utxo: UTXO,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt>;

    async fn call_get_minimum_deposit(
        &self,
        stream: StreamDenomination,
    ) -> alloy_contract::Result<U256>;

    async fn call_get_committee(
        &self,
        committee_id: CommitteeId,
    ) -> alloy_contract::Result<Committee>;

    async fn invoke_deposit_communication_data(
        &self,
        committee_id: CommitteeId,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt>;

    async fn invoke_deposit_aggregated_key(
        &self,
        committee_id: CommitteeId,
        aggregated_key: alloy_primitives::FixedBytes<32>,
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
    ) -> alloy_contract::Result<MemberKeys> {
        self.contract_instance
            .getMemberPublicKeys(member_address)
            .call()
            .await
    }

    async fn call_get_member_communication_data(
        &self,
        committee_id: CommitteeId,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<CommitteeRegistry::CommunicationData>> {
        self.contract_instance
            .getMemberCommunicationData(*committee_id, member_address)
            .call()
            .await
    }

    async fn call_get_member_funding_utxo(
        &self,
        stream_id: StreamId,
        member_address: Address,
    ) -> alloy_contract::Result<UTXO> {
        self.contract_instance
            .getMemberFundingUTXO(*stream_id, member_address)
            .call()
            .await
    }

    async fn invoke_apply_to_stream(
        &self,
        denomination: StreamDenomination,
        role: Role,
        public_keys: MemberRegistrationKeys,
        funding_utxo: UTXO,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt> {
        let stream = denomination.into_underlying();
        let role = role.into_underlying();

        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .applyToStream(stream, role, public_keys.clone(), funding_utxo.clone())
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
        // TODO(iago-2) fix this method to use the actual contract call

        // self.contract_instance.getMissingCommunicationDataCount
        //     .getMinimumDeposit(u8::from(stream))
        //     .call()
        //     .await

        // Temporary hardcoded minimum deposit (0.025 RBTC = 25000000000000000 wei)
        Ok(U256::from(25000000000000000u64))
    }

    async fn call_get_committee(
        &self,
        committee_id: CommitteeId,
    ) -> alloy_contract::Result<Committee> {
        self.contract_instance
            .getCommittee(*committee_id)
            .call()
            .await
    }

    async fn invoke_deposit_communication_data(
        &self,
        committee_id: CommitteeId,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt> {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .depositCommunicationData(*committee_id, communication_data.clone())
            },
            gas_bumps,
        )
        .await
    }

    async fn invoke_deposit_aggregated_key(
        &self,
        committee_id: CommitteeId,
        aggregated_key: alloy_primitives::FixedBytes<32>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<alloy_rpc_types::TransactionReceipt> {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .depositAggregatedKey(*committee_id, aggregated_key)
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
