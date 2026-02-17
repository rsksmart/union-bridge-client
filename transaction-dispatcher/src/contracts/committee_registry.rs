use alloy_primitives::{TxHash, U256};
use alloy_provider::Provider;
use common::types::CommitteeId;
use log::info;
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    self, Committee, CommitteeRegistryErrors, CommitteeRegistryInstance, MemberRegistrationKeys,
    UTXO,
};
use union_contracts::bindings::stream_manager::StreamManager::{Role, StreamDenomination};

use crate::contracts::common::send_tx_with_gas_bump;
pub(crate) use crate::contracts::interactions::apply_to_stream::ApplyToStreamInvoke;
pub(crate) use crate::contracts::interactions::deposit_aggregated_key::DepositAggregatedKeysInvoke;
pub(crate) use crate::contracts::interactions::deposit_communication_data::DepositCommunicationDataInvoke;
pub(crate) use crate::contracts::interactions::get_committee::GetCommitteeCall;
pub(crate) use crate::contracts::interactions::get_member_communication_data::GetMemberCommunicationDataCall;
use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;

#[cfg_attr(test, automock)]
pub trait CommitteeRegistryContractApi {
    async fn call_get_member_communication_data(
        &self,
        committee_id: CommitteeId,
        member_address: Address,
    ) -> alloy_contract::Result<Vec<CommitteeRegistry::CommunicationData>>;

    async fn invoke_apply_to_stream(
        &self,
        denomination: StreamDenomination,
        role: Role,
        public_keys: MemberRegistrationKeys,
        funding_utxo: UTXO,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<TxHash>;

    async fn call_get_committee(
        &self,
        committee_id: CommitteeId,
    ) -> alloy_contract::Result<Committee>;

    async fn invoke_deposit_communication_data(
        &self,
        committee_id: CommitteeId,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn invoke_deposit_aggregated_key(
        &self,
        committee_id: CommitteeId,
        aggregated_key: alloy_primitives::Bytes,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn call_is_whitelisted(&self, address: Address) -> alloy_contract::Result<bool>;

    async fn invoke_whitelist_address(
        &self,
        address: Address,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn invoke_whitelist_addresses(
        &self,
        addresses: Vec<Address>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;
}

#[derive(Clone)]
pub struct CommitteeRegistryContract<P: Provider> {
    contract_instance: CommitteeRegistryInstance<P>,
}

impl<P: Provider> CommitteeRegistryContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to CommitteeRegistry Contract @ {contract_address}");
        let contract_instance = CommitteeRegistry::new(contract_address, provider);
        CommitteeRegistryContract { contract_instance }
    }
}

impl<P: Provider> CommitteeRegistryContractApi for CommitteeRegistryContract<P> {
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

    async fn invoke_apply_to_stream(
        &self,
        denomination: StreamDenomination,
        role: Role,
        public_keys: MemberRegistrationKeys,
        funding_utxo: UTXO,
        gas_bumps: u8,
        value: U256,
    ) -> alloy_contract::Result<TxHash> {
        let stream = denomination.into_underlying();
        let role = role.into_underlying();

        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || {
                self.contract_instance
                    .applyToStream(stream, role, public_keys.clone(), funding_utxo.clone())
                    .value(value)
            },
            gas_bumps,
        )
        .await
    }

    async fn call_get_committee(
        &self,
        committee_id: CommitteeId,
    ) -> alloy_contract::Result<Committee> {
        self.contract_instance.getCommittee(*committee_id).call().await
    }

    async fn invoke_deposit_communication_data(
        &self,
        committee_id: CommitteeId,
        communication_data: Vec<CommitteeRegistry::CommunicationData>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
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
        aggregated_key: alloy_primitives::Bytes,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.depositAggregatedKey(*committee_id, aggregated_key.clone()),
            gas_bumps,
        )
        .await
    }

    async fn call_is_whitelisted(&self, address: Address) -> alloy_contract::Result<bool> {
        self.contract_instance.isWhitelisted(address).call().await
    }

    async fn invoke_whitelist_address(
        &self,
        address: Address,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.whitelistAddress(address),
            gas_bumps,
        )
        .await
    }

    async fn invoke_whitelist_addresses(
        &self,
        addresses: Vec<Address>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.whitelistAddresses(addresses.clone()),
            gas_bumps,
        )
        .await
    }
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<CommitteeRegistryErrors>()?;

    Some(match decoded_err {
        CommitteeRegistryErrors::MemberAlreadyDepositedCommunicationData(e) => {
            DomainErrors::MemberAlreadyDepositedCommunicationData(format!("{e:?}"))
        }
        CommitteeRegistryErrors::MemberInfoAlreadyDeposited(e) => {
            DomainErrors::MemberInfoAlreadyDeposited(format!("{e:?}"))
        }
        // Add other specific mappings here as needed
        _ => DomainErrors::CommitteeError(format!("{decoded_err:?}")),
    })
}
