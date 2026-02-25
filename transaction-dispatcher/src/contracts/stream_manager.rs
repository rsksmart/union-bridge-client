use alloy_primitives::{Bytes, U256};
use alloy_provider::Provider;
use log::info;
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::stream_manager::StreamManager::{
    self, Role, Stream, StreamDenomination, StreamManagerErrors, StreamManagerInstance,
};

use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;

#[cfg_attr(test, automock)]
pub trait StreamManagerContractApi {
    async fn call_get_minimum_deposit(
        &self,
        denomination: StreamDenomination,
        role: Role,
    ) -> alloy_contract::Result<U256>;

    async fn call_get_stream(&self, denomination: u64) -> alloy_contract::Result<Stream>;

    async fn call_get_enabler_script_pubkey(
        &self,
        stream_id: u64,
        packet_number: u64,
    ) -> alloy_contract::Result<Bytes>;
}

#[derive(Clone)]
pub struct StreamManagerContract<P: Provider> {
    contract_instance: StreamManagerInstance<P>,
}

impl<P: Provider> StreamManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to StreamManager Contract @ {contract_address}");
        let contract_instance = StreamManager::new(contract_address, provider);
        StreamManagerContract { contract_instance }
    }
}

impl<P: Provider> StreamManagerContractApi for StreamManagerContract<P> {
    async fn call_get_minimum_deposit(
        &self,
        denomination: StreamDenomination,
        role: Role,
    ) -> alloy_contract::Result<U256> {
        self.contract_instance
            .getMinimumDeposit(denomination.into_underlying(), role.into_underlying())
            .call()
            .await
    }

    async fn call_get_stream(&self, denomination: u64) -> alloy_contract::Result<Stream> {
        self.contract_instance.getStream(denomination).call().await
    }

    async fn call_get_enabler_script_pubkey(
        &self,
        stream_id: u64,
        packet_number: u64,
    ) -> alloy_contract::Result<Bytes> {
        self.contract_instance.getEnablerScriptPubKey(stream_id, packet_number).call().await
    }
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<StreamManagerErrors>();
    decoded_err.map(|e| match e {
        StreamManagerErrors::StreamNotFoundByDenomination(e) => {
            DomainErrors::StreamNotFoundByDenomination(format!("{e:?}"))
        }
        StreamManagerErrors::PacketOutOfBound(e) => {
            DomainErrors::PacketOutOfBound(format!("{e:?}"))
        }
        StreamManagerErrors::InvalidRole(e) => DomainErrors::InvalidRole(format!("{e:?}")),
        // TODO handle more based on needs
        _ => DomainErrors::UnhandledContractError(format!("{e:?}")),
    })
}

#[cfg(test)]
mod tests {
    use union_contracts::bindings::stream_manager::StreamManager::{
        PacketOutOfBound, StreamManagerErrors, StreamNotFoundByDenomination,
    };

    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;

    #[test]
    fn test_stream_not_found_by_denomination() {
        let expected_err =
            StreamManagerErrors::StreamNotFoundByDenomination(StreamNotFoundByDenomination {
                denomination: alloy_primitives::Uint::from(125),
            });

        let result = generate_contract_revert_error(&expected_err);
        matches!(result.into(), DomainErrors::StreamNotFoundByDenomination(_));
    }

    #[test]
    fn test_packet_out_of_bound() {
        let expected_err = StreamManagerErrors::PacketOutOfBound(PacketOutOfBound {
            packetNumber: alloy_primitives::U256::from(42),
        });

        let result = generate_contract_revert_error(&expected_err);
        matches!(result.into(), DomainErrors::PacketOutOfBound(_));
    }
}
