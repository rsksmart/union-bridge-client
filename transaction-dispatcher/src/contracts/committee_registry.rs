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
pub(crate) use crate::contracts::interactions::is_whitelisted::IsWhitelistedCall;
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

#[cfg(test)]
mod tests {
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        CommitteeRegistryErrors, MemberAlreadyDepositedCommunicationData,
        MemberInfoAlreadyDeposited,
    };

    use super::*;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;

    #[test]
    fn test_member_already_deposited_communication_data_error() {
        let err_data = CommitteeRegistryErrors::MemberAlreadyDepositedCommunicationData(
            MemberAlreadyDepositedCommunicationData {
                committeeId: 1,
                memberAddress: alloy_primitives::Address::default(),
                communicationDataLenght: alloy_primitives::U256::from(3),
            },
        );

        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::MemberAlreadyDepositedCommunicationData(_)));
    }

    #[test]
    fn test_member_info_already_deposited_error() {
        let err_data =
            CommitteeRegistryErrors::MemberInfoAlreadyDeposited(MemberInfoAlreadyDeposited {
                committeeId: 1,
                memberAddress: alloy_primitives::Address::default(),
            });

        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::MemberInfoAlreadyDeposited(_)));
    }

    #[test]
    fn test_unhandled_error_maps_to_committee_error() {
        use union_contracts::bindings::committee_registry::CommitteeRegistry::ERC1967InvalidImplementation;

        let err_data =
            CommitteeRegistryErrors::ERC1967InvalidImplementation(ERC1967InvalidImplementation {
                implementation: alloy_primitives::Address::default(),
            });

        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::CommitteeError(_)));
    }
}
