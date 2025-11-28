use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{DepositAggregatedKeyInput, DepositAggregatedKeyOutput};
use log::{error, info};

#[derive(Clone)]
pub(crate) struct DepositAggregatedKeysInvoke<C: CommitteeRegistryContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: CommitteeRegistryContractApi> DepositAggregatedKeysInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        DepositAggregatedKeysInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: DepositAggregatedKeyInput,
    ) -> Result<DepositAggregatedKeyOutput, DomainErrors> {
        info!("Init Deposit Aggregated Key for: {}", input.committee_id);

        let receipt = self
            .contract
            .invoke_deposit_aggregated_key(input.committee_id, input.aggregated_key, self.gas_bumps)
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "failed to deposit aggregated keys: {}",
                    e
                ))
            })?;

        let transaction_hash = format!("0x{:x}", receipt.transaction_hash);

        match receipt.status() {
            true => {
                info!(
                    "Deposit Aggregated Key successful at tx {}",
                    transaction_hash
                );
                Ok(DepositAggregatedKeyOutput { transaction_hash })
            }
            false => {
                error!("Deposit Aggregated Key failed at tx {}", transaction_hash);
                Err(DomainErrors::TransactionFailed(format!(
                    "DepositAggregatedKey transaction failed with receipt status false at tx {}",
                    transaction_hash
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use alloy_primitives::{Address, TxHash};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use common::types::CommitteeId;
    use mockall::predicate::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_deposit_aggregated_key_success() {
        // arrange
        let mut mock_contract = MockCommitteeRegistryContractApi::new();
        let committee_id: CommitteeId = 1.into();
        let aggregated_key = alloy_primitives::Bytes::from([1u8; 33].to_vec());
        let gas_bumps = 3u8;

        let expected_receipt = get_fake_receipt(
            true,
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        );
        mock_contract
            .expect_invoke_deposit_aggregated_key()
            .with(
                eq(committee_id.clone()),
                eq(aggregated_key.clone()),
                eq(gas_bumps),
            )
            .times(1)
            .returning(move |_, _, _| Ok(expected_receipt.clone()));

        let invoke = DepositAggregatedKeysInvoke::new(mock_contract, gas_bumps);
        let input = DepositAggregatedKeyInput {
            committee_id,
            aggregated_key,
        };

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
            .with(
                eq(committee_id.clone()),
                eq(aggregated_key.clone()),
                eq(gas_bumps),
            )
            .times(1)
            .returning(|_, _, _| {
                Err(alloy_contract::Error::TransportError(
                    alloy_transport::TransportError::local_usage_str("contract error"),
                ))
            });

        let invoke = DepositAggregatedKeysInvoke::new(mock_contract, gas_bumps);
        let input = DepositAggregatedKeyInput {
            committee_id,
            aggregated_key,
        };

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

    fn get_fake_receipt(status: bool, hash: &str) -> TransactionReceipt<ReceiptEnvelope<Log>> {
        let receipt = Receipt {
            status: status.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        };

        let envelope = ReceiptEnvelope::Eip1559(ReceiptWithBloom {
            receipt,
            logs_bloom: alloy_primitives::Bloom::ZERO,
        });

        TransactionReceipt {
            inner: envelope,
            transaction_hash: TxHash::from_str(hash).expect("transaction hash is invalid"),
            transaction_index: Some(0),
            block_hash: None,
            block_number: None,
            gas_used: 21_000,
            effective_gas_price: 0,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::default(),
            to: Some(Address::default()),
            contract_address: None,
        }
    }
}
