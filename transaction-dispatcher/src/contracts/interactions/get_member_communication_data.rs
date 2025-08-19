use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetCommunicationDataInput, GetCommunicationDataOutput};
use alloy_primitives::Address;
use log::info;

#[derive(Clone)]
pub(crate) struct GetMemberCommunicationDataCall<C: CommitteeRegistryContractApi> {
    contract: C,
}

impl<C: CommitteeRegistryContractApi> GetMemberCommunicationDataCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetMemberCommunicationDataCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetCommunicationDataInput,
    ) -> Result<GetCommunicationDataOutput, DomainErrors> {
        info!(
            "Init GetMemberCommunicationData for stream: {:?}, member: {:?}",
            input.stream_id, input.member_address
        );

        let communication_data = self
            .contract
            .call_get_member_communication_data(input.stream_id, input.member_address)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get member communication data: {}",
                    e
                ))
            })?;

        let count = communication_data.len();
        info!(
            "GetMemberCommunicationData successful, received {} communication entries",
            count
        );

        Ok(GetCommunicationDataOutput { communication_data })
    }
}

#[cfg(test)]
mod tests {
    use super::GetMemberCommunicationDataCall;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::GetCommunicationDataInput;
    use mockall::predicate::always;

    #[tokio::test]
    async fn test_get_member_communication_data_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        // Build a fake communication data structure: 2 committee members, each with 2 chunks
        #[allow(dead_code)]
        #[derive(Clone)]
        struct FakeCommunicationData {
            pub data: Vec<alloy_primitives::FixedBytes<32>>,
        }

        // Since Mock uses the trait return type (Vec<CommitteeRegistry::CommunicationData>),
        // we cannot construct the exact type from bindings here. Instead, we only validate
        // that our method processes the returned data when the mock returns Ok(vec![]).
        mock_instance
            .expect_call_get_member_communication_data()
            .with(always(), always())
            .returning(move |_, _| Ok(Vec::new()))
            .times(1);

        let interaction = GetMemberCommunicationDataCall::new_for_tests(mock_instance);

        let input = GetCommunicationDataInput {
            stream_id: 1u64,
            member_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                .parse()
                .expect("Invalid address"),
        };

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.communication_data.len(), 0);
    }

    #[tokio::test]
    async fn test_get_member_communication_data_contract_error() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        mock_instance
            .expect_call_get_member_communication_data()
            .with(always(), always())
            .returning(move |_, _| {
                Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: 3,
                        message: "Contract call failed".to_string().into(),
                        data: None,
                    }),
                ))
            })
            .times(1);

        let interaction = GetMemberCommunicationDataCall::new_for_tests(mock_instance);

        let input = GetCommunicationDataInput {
            stream_id: 1u64,
            member_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                .parse()
                .expect("Invalid address"),
        };

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::UnhandledContractError(_)
        );
    }

    impl GetMemberCommunicationDataCall<MockCommitteeRegistryContractApi> {
        pub(crate) fn new_for_tests(contract: MockCommitteeRegistryContractApi) -> Self {
            GetMemberCommunicationDataCall { contract }
        }
    }
}
