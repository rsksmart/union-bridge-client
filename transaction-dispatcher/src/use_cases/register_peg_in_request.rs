use crate::contracts::bitcoin_manager::BitcoinTransaction;
use crate::contracts::common::ContractInvokeReceipt;
use crate::contracts::peg_manager;
use crate::contracts::peg_manager::SolPegManager::PegInRequestTxSPVProof;
use crate::contracts::peg_manager::{PegManagerContractApi, PegManagerErrors};
use alloy_contract::Error::TransportError;
use alloy_provider::network::EthereumWallet;
use anyhow::Result;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct RegisterPegInInput {
    pub(crate) block_hash: String,
    pub(crate) btc_tx: BitcoinTransaction,
    pub(crate) merkle_branch_path: String,
    pub(crate) merkle_branch_hashes: Vec<String>,
}

pub(crate) struct RegisterPegInRequestInvoke<C: PegManagerContractApi> {
    contract: Arc<C>,
    signer: EthereumWallet,
}

impl<C: PegManagerContractApi> RegisterPegInRequestInvoke<C> {
    pub(crate) fn new(contract: Arc<C>, signer: EthereumWallet) -> Self {
        RegisterPegInRequestInvoke { contract, signer }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegInInput,
    ) -> Result<ContractInvokeReceipt, PegManagerErrors> {
        let parsed_input: PegInRequestTxSPVProof = input.try_into().map_err(|e| {
            error!("Failed to parse RegisterPegInInput: {}", e);
            // TODO(iago) do this validation outside, 404
            PegManagerErrors::InternalError
        })?;

        self.do_call(parsed_input.clone()).await?;

        let result = self
            .contract
            .register_peg_in_request_send(&self.signer, parsed_input)
            .await;
        match result {
            Ok(r) => {
                if r.status {
                    info!(
                        "RegisterPegInRequest successful at tx {}",
                        r.transaction_hash
                    );
                    Ok(r)
                } else {
                    error!(
                        "RegisterPegInRequest failed after successful call at tx {}",
                        r.transaction_hash
                    );
                    Err(PegManagerErrors::InternalError)
                }
            }
            Err(e) => {
                error!("Error sending PegInRequest: {}", e);
                Err(PegManagerErrors::InternalError)
            }
        }
    }

    async fn do_call(&self, parsed_input: PegInRequestTxSPVProof) -> Result<(), PegManagerErrors> {
        let result = self
            .contract
            .register_peg_in_request_call(parsed_input)
            .await;

        match result {
            Ok(_) => {
                debug!("RegisterPegInRequest call worked fine");
                Ok(())
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => Err(peg_manager::decode_contract_error(e)),
                None => {
                    error!("Missing ErrorPayload in PegManager error {:?}", err);
                    Err(PegManagerErrors::InternalError)
                }
            },
            Err(e) => {
                error!("Error calling PegManager: {:?}", e);
                Err(PegManagerErrors::InternalError)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::{
        BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut,
    };
    use crate::contracts::common::ContractInvokeReceipt;
    use crate::contracts::common::tests::generate_contract_expected_error;
    use crate::contracts::peg_manager::SolPegManager::{
        AlreadyRegisteredPegIn, SolPegManagerErrors, registerPegInRequestReturn,
    };
    use crate::contracts::peg_manager::{MockPegManagerContractApi, PegManagerErrors};
    use crate::use_cases::register_peg_in_request::{
        RegisterPegInInput, RegisterPegInRequestInvoke,
    };
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::RpcError::ErrorResp;
    use common::types::{BlockHash, BlockNumber};
    use std::sync::Arc;

    impl RegisterPegInRequestInvoke<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            RegisterPegInRequestInvoke {
                contract: Arc::new(contract),
                signer: Default::default(),
            }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();

        let input = get_base_input();

        let expected_receipt = ContractInvokeReceipt {
            block_number: BlockNumber::from(100),
            block_hash: BlockHash::try_from(
                "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c",
            )
            .expect("Failed to parse block hash"),
            transaction_hash: "0x4e3f8a2d39c1b872b77e8a5c9a24be8f1d489ea7cf2d38375f18b5b54e7df662"
                .to_string(),
            gas_used: 21000,
            status: true,
        };

        let receipt_to_return = expected_receipt.clone();

        mock.expect_register_peg_in_request_call()
            .returning(|_| Ok(registerPegInRequestReturn {}))
            .times(1);

        mock.expect_register_peg_in_request_send()
            .returning(move |_, _| Ok(receipt_to_return.clone()))
            .times(1);

        let invoke = RegisterPegInRequestInvoke::new_for_tests(mock);

        let result = invoke.run(input).await;
        assert!(result.is_ok());

        let result_receipt = result.unwrap();
        assert_eq!(result_receipt, expected_receipt);
    }

    // there are more errors that could be raised by the smart contract, but those are tested either on peg_manager.rs or bitcoin_manager.rs
    #[tokio::test]
    async fn test_run_call_fails() {
        let mut mock = MockPegManagerContractApi::new();

        let input = get_base_input();

        mock.expect_register_peg_in_request_call()
            .returning(move |_| {
                let expected_err =
                    SolPegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
                        btcTxHash:
                            "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                                .parse()
                                .expect("Failed to parse tx hash"),
                    });
                let expected_err_payload = generate_contract_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        mock.expect_register_peg_in_request_send().times(0);

        let invoke = RegisterPegInRequestInvoke::new_for_tests(mock);

        let result = invoke.run(input).await;
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            PegManagerErrors::AlreadyRegisteredPegIn
        );
    }

    #[tokio::test]
    async fn test_todo() {
        // TODO(iago) add more tests
    }

    fn get_base_input() -> RegisterPegInInput {
        let input = RegisterPegInInput {
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
}
