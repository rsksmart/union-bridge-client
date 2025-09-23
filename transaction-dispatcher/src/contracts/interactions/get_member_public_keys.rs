use crate::contracts::member_registry::MemberRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetMemberPublicKeysInput, GetMemberPublicKeysOutput, P2PAddressParser};
use log::info;

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
            "Init GetMemberPublicKeys for member: {:?}",
            input.member_address
        );

        let public_keys = self
            .contract
            .call_get_member_public_keys(input.member_address)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get member public keys: {}",
                    e
                ))
            })?;

        info!("GetMemberPublicKeys successful, retrieved member keys");

        // we store peer_id in communicationPubKey.rsaPublicKey, agreed with Fairgate
        let peer_id_bytes = &public_keys.communicationPubKey;
        let peer_id =
            P2PAddressParser::peer_id_from_member_contracts(peer_id_bytes).map_err(|e| {
                DomainErrors::InvalidPublicKey(format!(
                    "Failed to convert communication public keys to hex: {e}"
                ))
            })?;

        Ok(GetMemberPublicKeysOutput {
            public_keys: vec![
                format!("0x{:x}", public_keys.takePubKey),
                format!("0x{:x}", public_keys.covenantPubKey),
                peer_id,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::member_registry::MockMemberRegistryContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::P2PAddressParser;
    use alloy_primitives::{Address, FixedBytes};
    use mockall::predicate::always;
    use union_contracts::bindings::member_registry::MemberRegistry::{MemberKeys, RSAPublicKey};

    #[tokio::test]
    async fn test_get_member_public_keys_success() {
        let member_address: Address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            .parse()
            .expect("Invalid address");
        let input = GetMemberPublicKeysInput { member_address };

        let encoded_key = P2PAddressParser::peer_id_to_contracts(&"30820122300d06092a864886f70d01010105000382010f003082010a0282010100b0595a239c455f955ac2617061fadc0f3c532056da4a4ab4111b6581a62143e6c00b3041a00c290232fa65794ea0a55ca5f2ed3310ecbcab06a721d66e99a27e0d1b8a6afd8e395b741fbcf6cb73294eaeff43118f828f0118a4b5fdc95d472bcadaf2bc4d665e535ccd70b8ee5b82624794351a82c9f819d9a53638122228d1800d7d6561ae98183ae53c6cf23964c7eceeae95807db49a164cfbbc1ddc87a975fbe3d43545e8ce1bad2043cfe6a9aa3a7538ebdab8e6b900c94a691c1321d7c2d7f1a1beb3c3ef03686f7805ce938c92c8d5057cb5101cd51c1d97d7d3d4b9f13b7cb28bc5c4c5c9983a3062efc606b9c440021e1d5257d88d9c3ced0ac38f0203010001").unwrap();
        let expected_public_keys = MemberKeys {
            takePubKey: FixedBytes::from([1u8; 32]),
            covenantPubKey: FixedBytes::from([2u8; 32]),
            communicationPubKey: RSAPublicKey {
                rsaPublicKey: encoded_key.rsaPublicKey,
            },
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
        assert_eq!(
            output.public_keys[2],
            "30820122300d06092a864886f70d01010105000382010f003082010a0282010100b0595a239c455f955ac2617061fadc0f3c532056da4a4ab4111b6581a62143e6c00b3041a00c290232fa65794ea0a55ca5f2ed3310ecbcab06a721d66e99a27e0d1b8a6afd8e395b741fbcf6cb73294eaeff43118f828f0118a4b5fdc95d472bcadaf2bc4d665e535ccd70b8ee5b82624794351a82c9f819d9a53638122228d1800d7d6561ae98183ae53c6cf23964c7eceeae95807db49a164cfbbc1ddc87a975fbe3d43545e8ce1bad2043cfe6a9aa3a7538ebdab8e6b900c94a691c1321d7c2d7f1a1beb3c3ef03686f7805ce938c92c8d5057cb5101cd51c1d97d7d3d4b9f13b7cb28bc5c4c5c9983a3062efc606b9c440021e1d5257d88d9c3ced0ac38f0203010001"
        );
    }

    #[tokio::test]
    async fn test_get_member_public_keys_contract_error() {
        let member_address: Address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            .parse()
            .expect("Invalid address");
        let input = GetMemberPublicKeysInput { member_address };

        let mut mock_instance = MockMemberRegistryContractApi::new();
        mock_instance
            .expect_call_get_member_public_keys()
            .with(always())
            .returning(move |_| {
                Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: 3,
                        message: "Contract call failed".to_string().into(),
                        data: None,
                    }),
                ))
            })
            .times(1);

        let interaction = GetMemberPublicKeysCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::UnhandledContractError(_)
        );
    }

    impl GetMemberPublicKeysCall<MockMemberRegistryContractApi> {
        pub(crate) fn new_for_tests(contract: MockMemberRegistryContractApi) -> Self {
            GetMemberPublicKeysCall { contract }
        }
    }
}
