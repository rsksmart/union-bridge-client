use tracing::info;

use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{DepositAggregatedKeyInput, DepositAggregatedKeyOutput};

#[derive(Clone)]
pub(crate) struct DepositAggregatedKeysInvoke<C: CommitteeRegistryContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: CommitteeRegistryContractApi> DepositAggregatedKeysInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        DepositAggregatedKeysInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: DepositAggregatedKeyInput,
    ) -> Result<DepositAggregatedKeyOutput, DomainErrors> {
        info!("Init Deposit Aggregated Key for: {}", input.committee_id);

        let tx_hash = self
            .contract
            .invoke_deposit_aggregated_key(input.committee_id, input.aggregated_key, self.gas_bumps)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "failed to deposit aggregated keys: {e}"
                ))
            })?;

        let transaction_hash = format!("0x{tx_hash:x}");
        info!("Deposit Aggregated Key successful at tx {transaction_hash}");
        Ok(DepositAggregatedKeyOutput { transaction_hash })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::TxHash;
    use common_core::types::CommitteeId;
    use mockall::predicate::*;

    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;

    #[tokio::test]
    async fn test_deposit_aggregated_key_success() {
        // arrange
        let mut mock_contract = MockCommitteeRegistryContractApi::new();
        let committee_id: CommitteeId = 1.into();
        let aggregated_key = alloy_primitives::Bytes::from([1u8; 33].to_vec());
        let gas_bumps = 3u8;

        let expected_tx_hash = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        mock_contract
            .expect_invoke_deposit_aggregated_key()
            .with(eq(committee_id.clone()), eq(aggregated_key.clone()), eq(gas_bumps))
            .times(1)
            .returning(move |_, _, _| {
                Ok(TxHash::from_str(expected_tx_hash).expect("Failed to parse tx hash"))
            });

        let invoke = DepositAggregatedKeysInvoke::new(mock_contract, gas_bumps);
        let input = DepositAggregatedKeyInput { committee_id, aggregated_key };

        // act
        let result = invoke.run(input).await;

        // assert
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.transaction_hash.starts_with("0x"));
    }

    #[tokio::test]
    async fn test_deposit_aggregated_key_contract_error() {
        // arrange
        let mut mock_contract = MockCommitteeRegistryContractApi::new();
        let committee_id: CommitteeId = 1.into();
        let aggregated_key = alloy_primitives::Bytes::from([1u8; 33].to_vec());
        let gas_bumps = 3u8;

        mock_contract
            .expect_invoke_deposit_aggregated_key()
            .with(eq(committee_id.clone()), eq(aggregated_key.clone()), eq(gas_bumps))
            .times(1)
            .returning(|_, _, _| {
                Err(alloy_contract::Error::TransportError(
                    alloy_transport::TransportError::local_usage_str("contract error"),
                ))
            });

        let invoke = DepositAggregatedKeysInvoke::new(mock_contract, gas_bumps);
        let input = DepositAggregatedKeyInput { committee_id, aggregated_key };

        // act
        let result = invoke.run(input).await;

        // assert
        assert!(result.is_err());
        match result.unwrap_err() {
            DomainErrors::UnhandledContractError(msg) => {
                assert!(msg.contains("failed to deposit aggregated keys"));
            }
            _ => panic!("expected unhandledcontracterror"),
        }
    }
}
