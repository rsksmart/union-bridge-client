use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{AcceptPegInInput, AcceptPegInOutput};
use anyhow::Result;
use log::{error, info};
use union_contracts::bindings::pegmanager::PegManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct AcceptPegInRequestInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> AcceptPegInRequestInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        AcceptPegInRequestInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: AcceptPegInInput,
    ) -> Result<AcceptPegInOutput, DomainErrors> {
        info!("Init AcceptPegIn for: {:?}", input);

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!("Failed to parse AcceptPegInInput: {}", e))
        })?;

        let receipt = self
            .contract
            .accept_peg_in_request_send(parsed_input, self.gas_bumps)
            .await?;

        let result = match receipt.status() {
            true => {
                info!(
                    "AcceptPegInRequest successful at tx {}",
                    receipt.transaction_hash
                );
                AcceptPegInOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: true,
                }
            }
            false => {
                error!(
                    "AcceptPegInRequest failed at tx {}",
                    receipt.transaction_hash
                );
                AcceptPegInOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: false,
                }
            }
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::interactions::accept_peg_in_request::{
        AcceptPegInInput, AcceptPegInOutput, AcceptPegInRequestInvoke,
    };
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::{BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut};
    use alloy_primitives::{Address, Bloom, TxHash};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use std::str::FromStr;
    use union_contracts::bindings::pegmanager::PegManager::{
        AlreadyRegisteredAcceptPegIn, PegManagerErrors,
    };

    impl AcceptPegInRequestInvoke<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            AcceptPegInRequestInvoke {
                contract,
                gas_bumps: 3,
            }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();

        let input = get_base_input();

        let expected_receipt = AcceptPegInOutput {
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
            success: true,
        };

        let receipt_to_return = expected_receipt.clone();

        mock.expect_accept_peg_in_request_send()
            .returning(move |_, _| {
                Ok(get_fake_receipt(
                    true,
                    receipt_to_return.transaction_hash.as_str(),
                ))
            })
            .times(1);

        let invoke = AcceptPegInRequestInvoke::new_for_tests(mock);

        let result = invoke.run(input).await;
        assert!(result.is_ok());

        let result_receipt = result.unwrap();
        assert_eq!(result_receipt, expected_receipt);
    }

    // there are more errors that could be raised by the smart contract, but those are tested either on peg_manager.rs or bitcoin_manager.rs
    #[tokio::test]
    async fn test_run_fail_revert() {
        let mut mock = MockPegManagerContractApi::new();

        let input = get_base_input();

        mock.expect_accept_peg_in_request_send()
            .returning(move |_, _| {
                let expected_err =
                    PegManagerErrors::AlreadyRegisteredAcceptPegIn(AlreadyRegisteredAcceptPegIn {
                        btcTxHash:
                            "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                                .parse()
                                .expect("Failed to parse tx hash"),
                    });
                Err(generate_contract_revert_error(expected_err))
            })
            .times(1);

        mock.expect_accept_peg_in_request_send().times(0);

        let invoke = AcceptPegInRequestInvoke::new_for_tests(mock);

        let result = invoke.run(input).await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::AlreadyRegisteredAcceptPegIn(_)
        );
    }

    #[tokio::test]
    async fn test_run_fail_no_revert() {
        let mut mock = MockPegManagerContractApi::new();

        let input = get_base_input();

        let expected_receipt = AcceptPegInOutput {
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
            success: false,
        };

        let receipt_to_return = expected_receipt.clone();

        mock.expect_accept_peg_in_request_send()
            .returning(move |_, _| {
                Ok(get_fake_receipt(
                    false,
                    receipt_to_return.transaction_hash.as_str(),
                ))
            })
            .times(1);

        let invoke = AcceptPegInRequestInvoke::new_for_tests(mock);

        let result = invoke.run(input).await;
        assert!(result.is_ok());

        let result_receipt = result.unwrap();
        assert_eq!(result_receipt, expected_receipt);
    }

    fn get_base_input() -> AcceptPegInInput {
        let input = AcceptPegInInput {
            block_hash: "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9".to_string(),
            btc_tx: BitcoinTransaction {
                version: 1,
                inputs: vec![
                    BitcoinTransactionIn {
                        tx_id: "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2".to_string(),
                        v_out: 1694,
                        sequence: 4294967293,
                        script_sig: "".to_string(),
                    },
                ],
                outputs: vec![
                    BitcoinTransactionOut {
                        amount: 100000000,
                        script_pub_key: "0x512069d5a1d3da52fcaac436b735f6f75af910d3014f29d6eab4ba248a9786073d1f".to_string(),
                    }, BitcoinTransactionOut {
                        amount: 0,
                        script_pub_key: "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c8c72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1".to_string(),
                    },
                ],
                lock_time: 0,
            },
            merkle_branch_path: "0xFF6B0000".to_string(),
            merkle_branch_hashes: vec![
                "0x3fcef4a1ddf759a858190b89ecbd1ff3dffb49704e110b68baf5b5de7021910f".to_string(),
            ],
        };
        input
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
