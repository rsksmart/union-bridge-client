use log::info;

use crate::contracts::member_registry::MemberRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetMemberPublicKeysInput, GetMemberPublicKeysOutput, P2PAddressParser};

#[derive(Clone)]
pub(crate) struct GetMemberPublicKeysCall<C: MemberRegistryContractApi> {
    contract: C,
}

impl<C: MemberRegistryContractApi> GetMemberPublicKeysCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetMemberPublicKeysCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetMemberPublicKeysInput,
    ) -> Result<GetMemberPublicKeysOutput, DomainErrors> {
        info!(
            "Init GetMemberPublicKeys for member: {member_address:?}",
            member_address = input.member_address
        );

        let public_keys =
            self.contract.call_get_member_public_keys(input.member_address).await.map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get member public keys: {e}"
                ))
            })?;

        info!("GetMemberPublicKeys successful, retrieved member keys");

        // we store pubkey_hash in communicationPubKey.rsaPublicKey
        let pubkey_hash_bytes = &public_keys.communicationPubKey;
        let pubkey_hash = P2PAddressParser::pubkey_hash_from_member_contracts(pubkey_hash_bytes)
            .map_err(|e| {
                DomainErrors::InvalidPublicKey(format!(
                    "Failed to convert communication public keys to hex: {e}"
                ))
            })?;

        Ok(GetMemberPublicKeysOutput {
            public_keys: vec![
                format!("0x{:x}", public_keys.takePubKey),
                format!("0x{:x}", public_keys.covenantPubKey),
                pubkey_hash,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, FixedBytes};
    use mockall::predicate::always;
    use union_contracts::bindings::member_registry::MemberRegistry::{MemberKeys, RSAPublicKey};

    use super::*;
    use crate::contracts::member_registry::MockMemberRegistryContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::P2PAddressParser;

    #[tokio::test]
    async fn test_get_member_public_keys_success() {
        let member_address: Address =
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".parse().expect("Invalid address");
        let input = GetMemberPublicKeysInput { member_address };

        // SHA-256 hash (64 hex chars = 32 bytes)
        let test_pubkey_hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let encoded_key = P2PAddressParser::pubkey_hash_to_contracts(test_pubkey_hash).unwrap();
        let expected_public_keys = MemberKeys {
            takePubKey: FixedBytes::from([1u8; 32]),
            covenantPubKey: FixedBytes::from([2u8; 32]),
            communicationPubKey: RSAPublicKey { rsaPublicKey: encoded_key.rsaPublicKey },
        };

        let mut mock_instance = MockMemberRegistryContractApi::new();
        mock_instance
            .expect_call_get_member_public_keys()
            .with(always())
            .returning(move |_| Ok(expected_public_keys.clone()))
            .times(1);

        let interaction = GetMemberPublicKeysCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output.public_keys.len(), 3);
        assert_eq!(
            output.public_keys[0],
            "0x0101010101010101010101010101010101010101010101010101010101010101"
        );
        assert_eq!(
            output.public_keys[1],
            "0x0202020202020202020202020202020202020202020202020202020202020202"
        );
        assert_eq!(output.public_keys[2], test_pubkey_hash);
    }

    #[tokio::test]
    async fn test_get_member_public_keys_contract_error() {
        let member_address: Address =
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".parse().expect("Invalid address");
        let input = GetMemberPublicKeysInput { member_address };

        let mut mock_instance = MockMemberRegistryContractApi::new();
        mock_instance
            .expect_call_get_member_public_keys()
            .with(always())
            .returning(move |_| {
                Err(alloy_contract::Error::TransportError(alloy_json_rpc::RpcError::ErrorResp(
                    alloy_json_rpc::ErrorPayload {
                        code: 3,
                        message: "Contract call failed".to_string().into(),
                        data: None,
                    },
                )))
            })
            .times(1);

        let interaction = GetMemberPublicKeysCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::UnhandledContractError(_));
    }

    impl GetMemberPublicKeysCall<MockMemberRegistryContractApi> {
        pub(crate) fn new_for_tests(contract: MockMemberRegistryContractApi) -> Self {
            GetMemberPublicKeysCall { contract }
        }
    }
}
