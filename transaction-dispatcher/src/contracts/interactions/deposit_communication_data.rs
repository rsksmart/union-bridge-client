use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{DepositCommunicationDataInput, DepositCommunicationDataOutput};
use log::info;

#[derive(Clone)]
pub(crate) struct DepositCommunicationDataInvoke<C: CommitteeRegistryContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: CommitteeRegistryContractApi> DepositCommunicationDataInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        DepositCommunicationDataInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: DepositCommunicationDataInput,
    ) -> Result<DepositCommunicationDataOutput, DomainErrors> {
        info!(
            "Init DepositCommunicationData for stream: {}, data count: {}",
            input.committee_id,
            input.communication_data.len()
        );

        let tx_hash = self
            .contract
            .invoke_deposit_communication_data(
                input.committee_id,
                input.communication_data,
                self.gas_bumps,
            )
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to deposit communication data: {e}"
                ))
            })?;

        let transaction_hash = format!("0x{tx_hash:x}");
        info!("DepositCommunicationData successful at tx {transaction_hash}");
        Ok(DepositCommunicationDataOutput { transaction_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use alloy_primitives::TxHash;
    use mockall::predicate::always;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_deposit_communication_data_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        // create a mock receipt with a transaction hash
        let expected_tx_hash = "0x0101010101010101010101010101010101010101010101010101010101010101";

        mock_instance
            .expect_invoke_deposit_communication_data()
            .with(always(), always(), always())
            .returning(move |_, _, _| {
                Ok(TxHash::from_str(expected_tx_hash).expect("Failed to parse tx hash"))
            })
            .times(1);

        let interaction = DepositCommunicationDataInvoke::new(mock_instance, 3);

        let input = DepositCommunicationDataInput {
            committee_id: 1.into(),
            communication_data: vec![],
        };

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.transaction_hash, expected_tx_hash);
        // success field removed
    }

    #[tokio::test]
    async fn test_deposit_communication_data_contract_error() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        mock_instance
            .expect_invoke_deposit_communication_data()
            .with(always(), always(), always())
            .returning(|_, _, _| {
                Err(alloy_contract::Error::TransportError(
                    alloy_transport::TransportError::local_usage_str("test error"),
                ))
            })
            .times(1);

        let interaction = DepositCommunicationDataInvoke::new(mock_instance, 3);

        let input = DepositCommunicationDataInput {
            committee_id: 1.into(),
            communication_data: vec![],
        };

        let result = interaction.run(input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            DomainErrors::UnhandledContractError(msg) => {
                assert!(msg.contains("Failed to deposit communication data"));
            }
            _ => panic!("Expected UnhandledContractError"),
        }
    }
}
