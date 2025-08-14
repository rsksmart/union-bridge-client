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
            "Init DepositCommunicationData for stream: {:?}, data count: {}",
            input.stream_id,
            input.communication_data.len()
        );

        let receipt = self
            .contract
            .invoke_deposit_communication_data(
                input.stream_id,
                input.communication_data,
                self.gas_bumps,
            )
            .await
            .map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to deposit communication data: {}",
                    e
                ))
            })?;

        let transaction_hash = format!("0x{:x}", receipt.transaction_hash);
        let success = receipt.status();

        info!(
            "DepositCommunicationData completed with hash: {}, success: {}",
            transaction_hash, success
        );

        Ok(DepositCommunicationDataOutput {
            transaction_hash,
            success,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use alloy_primitives::TxHash;
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use mockall::predicate::always;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_deposit_communication_data_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        // create a mock receipt with a transaction hash
        let expected_tx_hash = "0x0101010101010101010101010101010101010101010101010101010101010101";
        let receipt = get_fake_receipt(true, expected_tx_hash);

        mock_instance
            .expect_invoke_deposit_communication_data()
            .with(always(), always(), always())
            .returning(move |_, _, _| Ok(receipt.clone()))
            .times(1);

        let interaction = DepositCommunicationDataInvoke::new(mock_instance, 3);

        let input = DepositCommunicationDataInput {
            stream_id: 1,
            communication_data: vec![],
        };

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.transaction_hash, expected_tx_hash);
        assert!(output.success);
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
            stream_id: 1,
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
            from: alloy_primitives::Address::default(),
            to: Some(alloy_primitives::Address::default()),
            contract_address: None,
        }
    }
}
