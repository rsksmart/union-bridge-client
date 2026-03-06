use alloy_provider::Provider;
use log::info;
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::member_registry::MemberRegistry::{
    self, MemberKeys, MemberRegistryErrors, MemberRegistryInstance,
};

pub(crate) use crate::contracts::interactions::get_member_public_keys::GetMemberPublicKeysCall;
use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;

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
        info!("Connecting to MemberRegistry Contract @ {contract_address}");
        let contract_instance = MemberRegistry::new(contract_address, provider);
        MemberRegistryContract { contract_instance }
    }
}

impl<P: Provider> MemberRegistryContractApi for MemberRegistryContract<P> {
    async fn call_get_member_public_keys(
        &self,
        member_address: Address,
    ) -> alloy_contract::Result<MemberKeys> {
        self.contract_instance.getMemberPublicKeys(member_address).call().await
    }
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<MemberRegistryErrors>()?;

    Some(match decoded_err {
        MemberRegistryErrors::MemberAlreadyRegisteredForStream(e) => {
            DomainErrors::MemberAlreadyRegisteredForStream(format!("{e:?}"))
        }
        // Add other specific mappings here as needed
        _ => DomainErrors::MemberRegistryError(format!("{decoded_err:?}")),
    })
}

#[cfg(test)]
mod tests {
    use union_contracts::bindings::member_registry::MemberRegistry::{
        MemberAlreadyRegisteredForStream, MemberRegistryErrors,
    };

    use super::*;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;

    #[test]
    fn test_member_already_registered_for_stream_error() {
        let err_data = MemberRegistryErrors::MemberAlreadyRegisteredForStream(
            MemberAlreadyRegisteredForStream {
                memberAddress: alloy_primitives::Address::default(),
                requestedStream: 1,
                requestedRole: 1,
                currentRole: 0,
            },
        );

        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::MemberAlreadyRegisteredForStream(_)));
    }

    #[test]
    fn test_unhandled_error_maps_to_member_registry_error() {
        use union_contracts::bindings::member_registry::MemberRegistry::ERC1967InvalidImplementation;

        let err_data =
            MemberRegistryErrors::ERC1967InvalidImplementation(ERC1967InvalidImplementation {
                implementation: alloy_primitives::Address::default(),
            });

        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::MemberRegistryError(_)));
    }
}
