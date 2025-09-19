use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;
use alloy_provider::Provider;
use log::info;
use union_contracts::bindings::member_registry::MemberRegistry::{
    self, MemberKeys,
};
use union_contracts::bindings::member_registry::MemberRegistry::{
    MemberRegistryErrors, MemberRegistryInstance,
};

pub(crate) use crate::contracts::interactions::get_member_public_keys::GetMemberPublicKeysCall;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait MemberRegistryContractApi {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<MemberKeys>;
}

#[derive(Clone)]
pub struct MemberRegistryContract<P: Provider> {
    contract_instance: MemberRegistryInstance<P>,
}

impl<P: Provider> MemberRegistryContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!(
            "Connecting to MemberRegistry Contract @ {}",
            contract_address
        );
        let contract_instance = MemberRegistry::new(contract_address, provider);
        MemberRegistryContract { contract_instance }
    }
}

impl<P: Provider> MemberRegistryContractApi for MemberRegistryContract<P> {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<MemberKeys> {
        self.contract_instance
            .getMemberPublicKeys(member_address)
            .call()
            .await
    }
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<MemberRegistryErrors>();
    decoded_err.map(|e| DomainErrors::MemberRegistryError(format!("{:?}", e)))
}