use union_contracts::bindings::stream_manager::StreamManager::StreamManagerErrors;

use crate::rsk_gateway::DomainErrors;

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<StreamManagerErrors>();
    decoded_err.map(|e| match e {
        StreamManagerErrors::StreamNotFoundByDenomination(e) => {
            DomainErrors::StreamNotFoundByDenomination(format!("{:?}", e))
        }
        StreamManagerErrors::PacketOutOfBound(e) => {
            DomainErrors::PacketOutOfBound(format!("{:?}", e))
        }
        // TODO handle more based on needs
        _ => DomainErrors::UnhandledContractError(format!("{:?}", e)),
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

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::StreamNotFoundByDenomination(_));
    }

    #[test]
    fn test_packet_out_of_bound() {
        let expected_err = StreamManagerErrors::PacketOutOfBound(PacketOutOfBound {
            packetNumber: alloy_primitives::U256::from(42),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::PacketOutOfBound(_));
    }
}
