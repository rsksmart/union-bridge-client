use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetMemberFundingUtxoInput, GetMemberFundingUtxoOutput};
use log::info;

#[derive(Clone)]
pub(crate) struct GetMemberFundingUtxoCall<C: CommitteeRegistryContractApi> {
    contract: C,
}

impl<C: CommitteeRegistryContractApi> GetMemberFundingUtxoCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetMemberFundingUtxoCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetMemberFundingUtxoInput,
    ) -> Result<GetMemberFundingUtxoOutput, DomainErrors> {
        info!(
            "Init GetMemberFundingUtxo for committee: {:?}, member: {:?}",
            input.stream_id, input.member_address
        );

        let utxo = self
            .contract
            .call_get_member_funding_utxo(input.stream_id, input.member_address)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get member funding utxo: {}",
                    e
                ))
            })?;

        info!(
            "GetMemberFundingUtxo successful, received utxo with txid: {:?}, outputIndex: {}",
            utxo.txid, utxo.outputIndex
        );

        Ok(GetMemberFundingUtxoOutput { utxo })
    }
}

#[cfg(test)]
mod tests {
    use super::GetMemberFundingUtxoCall;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::GetMemberFundingUtxoInput;
    use mockall::predicate::always;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::UTXO;

    #[tokio::test]
    async fn test_get_member_funding_utxo_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        let expected_utxo = UTXO {
            txid: alloy_primitives::FixedBytes::<32>::from([1u8; 32]),
            outputIndex: 0,
            amount: 1000,
        };

        mock_instance
            .expect_call_get_member_funding_utxo()
            .with(always(), always())
            .returning(move |_, _| Ok(expected_utxo.clone()))
            .times(1);

        let interaction = GetMemberFundingUtxoCall::new_for_tests(mock_instance);

        let input = GetMemberFundingUtxoInput {
            stream_id: 1.into(),
            member_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                .parse()
                .expect("Invalid address"),
        };

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(
            output.utxo.txid,
            alloy_primitives::FixedBytes::<32>::from([1u8; 32])
        );
        assert_eq!(output.utxo.outputIndex, 0);
        assert_eq!(output.utxo.amount, 1000);
    }

    #[tokio::test]
    async fn test_get_member_funding_utxo_contract_error() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        mock_instance
            .expect_call_get_member_funding_utxo()
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

        let interaction = GetMemberFundingUtxoCall::new_for_tests(mock_instance);

        let input = GetMemberFundingUtxoInput {
            stream_id: 1.into(),
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

    impl GetMemberFundingUtxoCall<MockCommitteeRegistryContractApi> {
        pub(crate) fn new_for_tests(contract: MockCommitteeRegistryContractApi) -> Self {
            GetMemberFundingUtxoCall { contract }
        }
    }
}
