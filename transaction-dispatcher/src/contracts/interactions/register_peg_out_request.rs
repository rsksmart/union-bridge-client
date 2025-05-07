use crate::{
    contracts::peg_manager::PegManagerContractApi,
    rsk_gateway::DomainErrors,
    types::{RegisterPegOutInput, RegisterPegOutOutput},
};
use alloy_primitives::FixedBytes;
use anyhow::Result;
use log::{debug, error, info};

pub struct RegisterPegOutRequestInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterPegOutRequestInvoke<C> {
    pub fn new(contract: C, gas_bumps: u8) -> Self {
        Self {
            contract,
            gas_bumps,
        }
    }

    pub async fn run(
        &self,
        input: RegisterPegOutInput,
    ) -> Result<RegisterPegOutOutput, DomainErrors> {
        info!("Init RegisterPegOut for: {:?}", input);

        let msg_value = input.amount_in_wei;

        let usr_pub_key: FixedBytes<33> =
            input.usr_pub_key.parse::<FixedBytes<33>>().map_err(|e| {
                DomainErrors::InvalidPublicKey(format!("Failed to parse usr_pub_key: {}", e))
            })?;

        let batch_flag = input.batch_flag;

        debug!(
            "Calling register_peg_out_request_send: value = {}, usr_pub_key = {:?}, batch_flag = {}, gas_bumps = {}",
            msg_value, usr_pub_key, batch_flag, self.gas_bumps
        );

        let receipt = self
            .contract
            .register_peg_out_request_send(msg_value, usr_pub_key, batch_flag, self.gas_bumps)
            .await?;

        let result = if receipt.status() {
            info!(
                "RegisterPegOutRequest successful at tx {}",
                receipt.transaction_hash
            );
            RegisterPegOutOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
                success: true,
            }
        } else {
            error!(
                "RegisterPegOutRequest failed at tx {}",
                receipt.transaction_hash
            );
            RegisterPegOutOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
                success: false,
            }
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{
            interactions::register_peg_out_request::{
                RegisterPegOutInput, RegisterPegOutOutput, RegisterPegOutRequestInvoke,
            },
            peg_manager::MockPegManagerContractApi,
        },
        rsk_gateway::DomainErrors,
    };
    use alloy_primitives::{Bloom, TxHash};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use std::str::FromStr;

    impl RegisterPegOutRequestInvoke<MockPegManagerContractApi> {
        fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            RegisterPegOutRequestInvoke {
                contract,
                gas_bumps: 3,
            }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();
        let expected = RegisterPegOutOutput {
            transaction_hash: "0xfeedfacecafebeef000000000000000000000000000000000000000000000000"
                .to_string(),
            success: true,
        };
        let receipt_return = expected.clone();

        mock.expect_register_peg_out_request_send()
            .returning(move |_, _, _, _| {
                Ok(get_fake_receipt(true, &receipt_return.transaction_hash))
            })
            .times(1);

        let invoke = RegisterPegOutRequestInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_run_fail_no_revert() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();

        let expected = RegisterPegOutOutput {
            transaction_hash: "0xdeadbeefdeadbeef000000000000000000000000000000000000000000000000"
                .to_string(),
            success: false,
        };
        let receipt_return = expected.clone();

        mock.expect_register_peg_out_request_send()
            .returning(move |_, _, _, _| {
                Ok(get_fake_receipt(false, &receipt_return.transaction_hash))
            })
            .times(1);

        let invoke = RegisterPegOutRequestInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_run_invalid_pub_key_length() {
        let mut mock = MockPegManagerContractApi::new();
        // should never hit the contract if parse fails
        mock.expect_register_peg_out_request_send().times(0);

        let invoke = RegisterPegOutRequestInvoke::new_for_tests(mock);
        let bad_input = RegisterPegOutInput {
            amount_in_wei: 1_000,
            usr_pub_key: "not-a-hex-key".to_string(),
            batch_flag: false,
        };

        let err = invoke.run(bad_input).await.err().unwrap();
        match err {
            DomainErrors::InvalidPublicKey(msg) => {
                assert!(msg.contains("Failed to parse usr_pub_key"))
            }
            _ => panic!("expected InvalidPublicKey, got {:?}", err),
        }
    }

    fn get_base_input() -> RegisterPegOutInput {
        let usr_pub_key = format!("0x{}", "01".repeat(33));
        RegisterPegOutInput {
            amount_in_wei: 1_234_567,
            usr_pub_key,
            batch_flag: true,
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
