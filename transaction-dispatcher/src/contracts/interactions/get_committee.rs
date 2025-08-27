use crate::{
    contracts::committee_registry::CommitteeRegistryContractApi,
    rsk_gateway::DomainErrors,
    types::{GetCommitteeInput, GetCommitteeOutput},
};
use log::info;

#[derive(Clone)]
pub(crate) struct GetCommitteeCall<C: CommitteeRegistryContractApi> {
    contract: C,
}

impl<C: CommitteeRegistryContractApi> GetCommitteeCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        Self { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetCommitteeInput,
    ) -> Result<GetCommitteeOutput, DomainErrors> {
        info!("Init GetCommittee for committee_id: {}", input.committee_id);

        let committee = self
            .contract
            .call_get_committee(input.committee_id)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!("Failed to get committee: {}", e))
            })?;

        info!("GetCommittee successful, found committee: {:?}", committee);

        Ok(GetCommitteeOutput { committee })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use alloy_primitives::{Address, FixedBytes, U256, address};
    use mockall::predicate::always;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        Committee, CommitteeMember,
    };

    #[tokio::test]
    async fn test_get_committee_success() {
        let mut mock = MockCommitteeRegistryContractApi::new();

        let expected_committee = sample_committee();
        let expected_committee_clone = expected_committee.clone();

        mock.expect_call_get_committee()
            .with(always())
            .returning(move |_| Ok(expected_committee_clone.clone()))
            .times(1);

        let interaction = GetCommitteeCall::new(mock);

        let input = GetCommitteeInput {
            committee_id: 123.into(),
        };

        let result = interaction.run(input).await;
        assert!(result.is_ok());

        let out = result.unwrap();
        assert_eq!(
            out.committee.aggregatedKey,
            expected_committee.aggregatedKey
        );
        assert_eq!(
            out.committee.leaderAddress,
            expected_committee.leaderAddress
        );
        assert_eq!(
            out.committee.operatorTakeIndex,
            expected_committee.operatorTakeIndex
        );
        assert_eq!(out.committee.members.len(), 1);
        assert_eq!(
            out.committee.members[0].memberAddress,
            expected_committee.members[0].memberAddress
        );
    }

    #[tokio::test]
    async fn test_get_committee_contract_error() {
        let mut mock = MockCommitteeRegistryContractApi::new();

        mock.expect_call_get_committee()
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

        let interaction = GetCommitteeCall::new(mock);

        let input = GetCommitteeInput {
            committee_id: 456.into(),
        };

        let result = interaction.run(input).await;
        assert!(result.is_err());

        match result.err().unwrap() {
            DomainErrors::UnhandledContractError(msg) => {
                assert!(
                    msg.contains("Failed to get committee"),
                    "unexpected error message: {msg}"
                );
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    fn sample_committee() -> Committee {
        let aggregated_key = FixedBytes::<32>::from([0x11u8; 32]);
        let leader: Address = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
        let members = vec![CommitteeMember {
            memberAddress: leader,
            role: 1.into(),
        }];
        let operator_take_index = U256::from(42u64);

        Committee {
            aggregatedKey: aggregated_key,
            members,
            leaderAddress: leader,
            operatorTakeIndex: operator_take_index,
            createdAt: Default::default(),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 0,
            fundingUTXOs: vec![],
        }
    }
}
