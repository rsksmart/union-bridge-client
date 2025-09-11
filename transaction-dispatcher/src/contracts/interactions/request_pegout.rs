use crate::{
    contracts::peg_manager::PegManagerContractApi,
    rsk_gateway::DomainErrors,
    types::{RequestPegoutInput, RequestPegoutOutput},
};
use alloy_primitives::FixedBytes;
use anyhow::Result;
use log::{debug, error, info};

#[derive(Clone)]
pub struct TryPegoutInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> TryPegoutInvoke<C> {
    pub fn new(contract: C, gas_bumps: u8) -> Self {
        Self {
            contract,
            gas_bumps,
        }
    }

    pub async fn run(
        &self,
        input: RequestPegoutInput,
    ) -> Result<RequestPegoutOutput, DomainErrors> {
        info!("Init Pegout request for: {:?}", input);

        let msg_value = input.amount_in_wei;

        let usr_pub_key: FixedBytes<33> =
            input.usr_pub_key.parse::<FixedBytes<33>>().map_err(|e| {
                DomainErrors::InvalidCompressedPubKey(format!("Failed to parse usr_pub_key: {}", e))
            })?;

        debug!(
            "Calling invoke_request_pegout: value = {}, usr_pub_key = {:?}, gas_bumps = {}",
            msg_value, usr_pub_key, self.gas_bumps
        );

        let receipt = self
            .contract
            .invoke_request_pegout(msg_value, usr_pub_key, self.gas_bumps)
            .await?;

        match receipt.status() {
            true => {
                info!(
                    "Pegout Request successful at tx {}",
                    receipt.transaction_hash
                );
                Ok(RequestPegoutOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                })
            }
            false => {
                error!("Pegout request failed at tx {}", receipt.transaction_hash);
                Err(DomainErrors::TransactionFailed(format!(
                    "RequestPegout transaction failed with receipt status false at tx {}",
                    receipt.transaction_hash
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{
            interactions::request_pegout::{
                RequestPegoutInput, RequestPegoutOutput, TryPegoutInvoke,
            },
            peg_manager::MockPegManagerContractApi,
        },
        rsk_gateway::DomainErrors,
    };
    use alloy_primitives::{Bloom, TxHash};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use std::str::FromStr;

    impl TryPegoutInvoke<MockPegManagerContractApi> {
        fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            TryPegoutInvoke {
                contract,
                gas_bumps: 3,
            }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();
        let expected = RequestPegoutOutput {
            transaction_hash: "0xfeedfacecafebeef000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let receipt_return = expected.clone();

        mock.expect_invoke_request_pegout()
            .returning(move |_, _, _| Ok(get_fake_receipt(true, &receipt_return.transaction_hash)))
            .times(1);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_run_fail_no_revert() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();

        let expected_tx_hash = "0xdeadbeefdeadbeef000000000000000000000000000000000000000000000000";

        mock.expect_invoke_request_pegout()
            .returning(move |_, _, _| Ok(get_fake_receipt(false, expected_tx_hash)))
            .times(1);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            DomainErrors::TransactionFailed(msg) => {
                assert!(msg.contains("RequestPegout transaction failed"));
                assert!(msg.contains(expected_tx_hash));
            }
            _ => panic!("Expected TransactionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_run_invalid_pub_key_length() {
        let mut mock = MockPegManagerContractApi::new();
        // should never hit the contract if parse fails
        mock.expect_invoke_request_pegout().times(0);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let bad_input = RequestPegoutInput {
            amount_in_wei: 1_000,
            usr_pub_key: "not-a-hex-key".to_string(),
        };

        let err = invoke.run(bad_input).await.err().unwrap();
        match err {
            DomainErrors::InvalidCompressedPubKey(msg) => {
                assert!(msg.contains("Failed to parse usr_pub_key"))
            }
            _ => panic!("expected InvalidPublicKey, got {:?}", err),
        }
    }

    fn get_base_input() -> RequestPegoutInput {
        let usr_pub_key = format!("0x{}", "01".repeat(33));
        RequestPegoutInput {
            amount_in_wei: 1_234_567,
            usr_pub_key,
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
            logs_bloom: Bloom::ZERO,
        });
        TransactionReceipt {
            inner: envelope,
            transaction_hash: TxHash::from_str(hash).expect("invalid tx hash"),
            transaction_index: Some(0),
            block_hash: None,
            block_number: None,
            gas_used: 21_000,
            effective_gas_price: 0,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Default::default(),
            to: Some(Default::default()),
            contract_address: None,
        }
    }
}
