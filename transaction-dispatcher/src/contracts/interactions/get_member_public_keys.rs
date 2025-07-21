use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::GetMemberPublicKeysOutput;
use alloy_primitives::Address;
use log::info;

#[derive(Clone)]
pub(crate) struct GetMemberPublicKeysCall<C: CommitteeRegistryContractApi> {
    contract: C,
    member_address: Address,
}

impl<C: CommitteeRegistryContractApi> GetMemberPublicKeysCall<C> {
    pub(crate) fn new(contract: C, member_address: Address) -> Self {
        GetMemberPublicKeysCall {
            contract,
            member_address,
        }
    }

    pub(crate) async fn run(&self) -> Result<GetMemberPublicKeysOutput, DomainErrors> {
        info!(
            "Init GetMemberPublicKeys for member: {:?}",
            self.member_address
        );

        let public_keys = self
            .contract
            .get_member_public_keys_call(self.member_address)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get member public keys: {}",
                    e
                ))
            })?;

        info!(
            "GetMemberPublicKeys successful, found {} public keys",
            public_keys.len()
        );

        Ok(GetMemberPublicKeysOutput {
            public_keys: public_keys
                .into_iter()
                .map(|key| format!("0x{:x}", key))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use crate::contracts::interactions::get_member_public_keys::GetMemberPublicKeysCall;
    use crate::rsk_gateway::DomainErrors;
    use mockall::predicate::always;

    #[tokio::test]
    async fn test_get_member_public_keys_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        let expected_public_keys = vec![
            alloy_primitives::FixedBytes::from([1u8; 32]),
            alloy_primitives::FixedBytes::from([2u8; 32]),
        ];

        mock_instance
            .expect_get_member_public_keys_call()
            .with(always())
            .returning(move |_| Ok(expected_public_keys.clone()))
            .times(1);

        let interaction = GetMemberPublicKeysCall::new_for_tests(mock_instance);

        let result = interaction.run().await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output.public_keys.len(), 2);
        assert_eq!(
            output.public_keys[0],
            "0x0101010101010101010101010101010101010101010101010101010101010101"
        );
        assert_eq!(
            output.public_keys[1],
            "0x0202020202020202020202020202020202020202020202020202020202020202"
        );
    }

    #[tokio::test]
    async fn test_get_member_public_keys_contract_error() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        mock_instance
            .expect_get_member_public_keys_call()
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

        let result = interaction.run().await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::UnhandledContractError(_)
        );
    }

    impl GetMemberPublicKeysCall<MockCommitteeRegistryContractApi> {
        pub(crate) fn new_for_tests(contract: MockCommitteeRegistryContractApi) -> Self {
            GetMemberPublicKeysCall {
                contract,
                member_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                    .parse()
                    .expect("Invalid address"),
            }
        }
    }
}
