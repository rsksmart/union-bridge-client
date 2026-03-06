use anyhow::Result;
use log::info;
use union_contracts::bindings::pegout_manager::PegoutManager::BtcTxSPVProof;

use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterPegoutInput, RegisterPegoutOutput};

#[derive(Clone)]
pub(crate) struct RegisterPegoutInvoke<C: PegoutManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegoutManagerContractApi> RegisterPegoutInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterPegoutInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput, DomainErrors> {
        info!("Init RegisterPegout for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!("Failed to parse RegisterPegoutInput: {e}"))
        })?;

        let tx_hash = self.contract.invoke_register_user_take(parsed_input, self.gas_bumps).await?;

        info!("invoke_register_pegout successful at tx {tx_hash}");
        Ok(RegisterPegoutOutput { transaction_hash: tx_hash.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::TxHash;
    use union_contracts::bindings::peg_manager::PegManager::{
        PegManagerErrors, PeginAlreadyRequested,
    };

    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::interactions::register_pegout::{
        RegisterPegoutInvoke, RegisterPegoutOutput,
    };
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use crate::rsk_gateway::DomainErrors;
    use crate::types::{
        BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut, RegisterPegoutInput,
    };

    impl RegisterPegoutInvoke<MockPegManagerContractApi> {
        fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            RegisterPegoutInvoke { contract, gas_bumps: 3 }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();

        let expected = RegisterPegoutOutput {
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
        };
        let receipt_to_return = expected.clone();

        mock.expect_invoke_register_pegout()
            .returning(move |_, _| {
                Ok(TxHash::from_str(&receipt_to_return.transaction_hash)
                    .expect("Failed to parse tx hash"))
            })
            .times(1);

        let invoke = RegisterPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[tokio::test]
    async fn test_run_fail_revert() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();

        mock.expect_invoke_register_pegout()
            .returning(move |_, _| {
                let err = PegManagerErrors::PeginAlreadyRequested(PeginAlreadyRequested {
                    btcTxid: "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                        .parse()
                        .expect("Failed to parse tx hash"),
                });
                Err(generate_contract_revert_error(&err))
            })
            .times(1);

        let invoke = RegisterPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DomainErrors::PeginAlreadyRequested(_)));
    }

    #[tokio::test]
    async fn test_run_fail_no_revert() {
        let mut mock = MockPegManagerContractApi::new();
        let input = get_base_input();

        mock.expect_invoke_register_pegout()
            .returning(move |_, _| {
                Err(alloy_contract::Error::TransportError(
                    alloy_transport::TransportError::local_usage_str("transaction failed"),
                ))
            })
            .times(1);

        let invoke = RegisterPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            DomainErrors::NoRevertError(msg) => assert!(msg.contains("transaction failed")),
            other => panic!("Expected NoRevertError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_fail_parse() {
        let mock = MockPegManagerContractApi::new();
        let mut input = get_base_input();
        input.block_hash = "not_valid_hex".to_string();

        let invoke = RegisterPegoutInvoke::new_for_tests(mock);
        let result = invoke.run(input).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DomainErrors::InvalidBtcTxSpvProof(_)));
    }

    fn get_base_input() -> RegisterPegoutInput {
        RegisterPegoutInput {
            block_hash: "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9"
                .to_string(),
            btc_tx: BitcoinTransaction {
                version: 1,
                inputs: vec![BitcoinTransactionIn {
                    tx_id: "0x360b81785dc7c2f40627fea364676dbb73e6276683caffd9f906b0e0bd36b3d2"
                        .to_string(),
                    v_out: 1694,
                    sequence: 4_294_967_293,
                    script_sig: String::new(),
                }],
                outputs: vec![
                    BitcoinTransactionOut {
                        amount: 100_000_000,
                        script_pub_key: "0x512069d5a1d3da52fcaac436b735f6f75af910d3014f29d6eab4ba248a9786073d1f".to_string(),
                    },
                    BitcoinTransactionOut {
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
        }
    }
}
