use crate::{
    contracts::pegout_manager::PegoutManagerContractApi,
    rsk_gateway::DomainErrors,
    types::{RequestPegoutInput, RequestPegoutOutput},
};
use alloy_primitives::FixedBytes;
use anyhow::Result;
use log::{debug, info};

#[derive(Clone)]
pub struct TryPegoutInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> TryPegoutInvoke<C> {
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
        info!("Init Pegout request for: {input:?}");

        let msg_value = input.amount_in_wei;

        let usr_pub_key: FixedBytes<33> =
            input.usr_pub_key.parse::<FixedBytes<33>>().map_err(|e| {
                DomainErrors::InvalidCompressedPubKey(format!("Failed to parse usr_pub_key: {e}"))
            })?;

        debug!(
            "Calling invoke_try_pegout: value = {msg_value}, usr_pub_key = {usr_pub_key:?}, gas_bumps = {}",
            self.gas_bumps
        );

        let receipt = self
            .contract
            .invoke_try_pegout(msg_value, usr_pub_key, self.gas_bumps)
            .await?;

        let tx_hash = receipt.transaction_hash();
        info!("Pegout Request successful at tx {tx_hash}");
        Ok(RequestPegoutOutput {
            transaction_hash: tx_hash.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{
            interactions::request_pegout::{
                RequestPegoutInput, RequestPegoutOutput, TryPegoutInvoke,
            },
            pegout_manager::MockPegoutManagerContractApi,
        },
        rsk_gateway::DomainErrors,
    };
    use alloy_primitives::TxHash;
    use alloy_rpc_types::TransactionReceipt;
    use std::str::FromStr;

    impl TryPegoutInvoke<MockPegoutManagerContractApi> {
        fn new_for_tests(contract: MockPegoutManagerContractApi) -> Self {
            TryPegoutInvoke {
                contract,
                gas_bumps: 3,
            }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegoutManagerContractApi::new();
        let input = get_base_input();
        let expected = RequestPegoutOutput {
            transaction_hash: "0xfeedfacecafebeef000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let receipt_return = expected.clone();

        mock.expect_invoke_try_pegout()
            .returning(move |_, _, _| {
                Ok(create_fake_receipt(
                    true,
                    &receipt_return.transaction_hash,
                ))
            })
            .times(1);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_run_fail_no_revert() {
        let mut mock = MockPegoutManagerContractApi::new();
        let input = get_base_input();

        let expected_tx_hash = "0xdeadbeefdeadbeef000000000000000000000000000000000000000000000000";

        mock.expect_invoke_try_pegout()
            .returning(move |_, _, _| {
                Ok(create_fake_receipt(false, expected_tx_hash))
            })
            .times(1);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_invalid_pub_key_length() {
        let mut mock = MockPegoutManagerContractApi::new();
        // should never hit the contract if parse fails
        mock.expect_invoke_try_pegout().times(0);

        let invoke = TryPegoutInvoke::new_for_tests(mock);
        let bad_input = RequestPegoutInput {
            amount_in_wei: 1_000,
            usr_pub_key: "not-a-hex-key".to_string(),
        };

        let err = invoke.run(bad_input).await.err().unwrap();
        match err {
            DomainErrors::InvalidCompressedPubKey(msg) => {
                assert!(msg.contains("Failed to parse usr_pub_key"));
            }
            _ => panic!("expected InvalidPublicKey, got {err:?}"),
        }
    }

    fn get_base_input() -> RequestPegoutInput {
        let usr_pub_key = format!("0x{}", "01".repeat(33));
        RequestPegoutInput {
            amount_in_wei: 1_234_567,
            usr_pub_key,
        }
    }

    fn create_fake_receipt(status: bool, tx_hash_str: &str) -> TransactionReceipt {
        use alloy_primitives::{Address, Bloom};
        use alloy_rpc_types::{Receipt, ReceiptEnvelope, ReceiptWithBloom};

        let tx_hash = TxHash::from_str(tx_hash_str).expect("Failed to parse tx hash");
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
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: None,
            block_number: None,
            gas_used: 21_000,
            effective_gas_price: 1_000_000_000,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::from([3u8; 20]),
            to: Some(Address::from([4u8; 20])),
            contract_address: None,
        }
    }
}
