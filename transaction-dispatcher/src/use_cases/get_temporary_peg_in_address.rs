use crate::contracts::peg_manager;
use crate::contracts::peg_manager::{PegManagerContractApi, PegManagerErrors};
use alloy_contract::Error::TransportError;
use alloy_primitives::{Address, FixedBytes};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// TODO(iago) generate Try_From for the input struct like in the other cases

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PegInAddressInput {
    rootstock_deposit_address: String,
    value: u64,
    btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PegInAddressOutput {
    address: String,
}

pub(crate) struct GetTemporaryPegInAddressCall<C: PegManagerContractApi> {
    contract: Arc<C>,
}

impl<C: PegManagerContractApi> GetTemporaryPegInAddressCall<C> {
    pub(crate) fn new(contract: Arc<C>) -> Self {
        GetTemporaryPegInAddressCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors> {
        let rootstock_deposit_address: Address = input
            .rootstock_deposit_address
            .parse::<Address>()
            .map_err(|e| {
                error!("Failed to parse rootstock_deposit_address: {}", e);
                PegManagerErrors::InvalidAddress
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> = input
            .btc_reimbursement_pub_key
            .parse::<FixedBytes<32>>()
            .map_err(|e| {
                error!("Failed to parse btc_reimbursement_pub_key: {}", e);
                PegManagerErrors::InvalidPublicKey
            })?;

        let result = self
            .contract
            .get_temporary_peg_in_address_call(
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
            )
            .await;

        match result {
            Ok(data) => {
                debug!(
                    "Bitcoin Deposit Address for {:?}: {}",
                    input, data.bitcoinDepositAddress
                );

                Ok(PegInAddressOutput {
                    address: data.bitcoinDepositAddress.to_string(),
                })
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

#[cfg(all(test, feature = "testing"))]
mod tests {
    use crate::contracts::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, InvalidAddress, InvalidPublicKey,
    };
    use crate::contracts::peg_manager::SolPegManager::{
        AlreadyRegisteredPegIn, SolPegManagerErrors, StreamNotFoundByDenomination,
        getTemporaryPegInAddressReturn,
    };
    use crate::contracts::peg_manager::{MockPegManagerContractApi, PegManagerErrors};
    use crate::use_cases::get_temporary_peg_in_address::{
        GetTemporaryPegInAddressCall, PegInAddressInput,
    };
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::ErrorPayload;
    use alloy_json_rpc::RpcError::ErrorResp;
    use alloy_primitives::Address;
    use alloy_primitives::FixedBytes;
    use alloy_sol_types::SolInterface;
    use mockall::predicate::{always, eq};
    use std::sync::Arc;

    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;

    const ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

    #[cfg(test)]
    impl GetTemporaryPegInAddressCall<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            GetTemporaryPegInAddressCall {
                contract: Arc::new(contract),
            }
        }
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };
        let expected_deposit_address = "0xfake0deposit0address".to_string();
        let output = getTemporaryPegInAddressReturn {
            bitcoinDepositAddress: expected_deposit_address.clone(),
        };

        mock_instance
            .expect_get_temporary_peg_in_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| Ok(output.clone()))
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().address, expected_deposit_address);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_preliminary_validation() {
        let mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_smart_contract_raised() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            // it has to be valid here in order to pass the preliminary validation (non SC)
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_peg_in_address_call()
            .with(
                always(),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidAddress(InvalidAddress {
                    _address: Address::default(),
                });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_preliminary_validation() {
        let mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: "0xinvalid_pub_key".to_string(),
        };

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_smart_contract_raised() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_peg_in_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
                    publicKey: FixedBytes::<32>::default(),
                });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_internal_server_error() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PegInAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_peg_in_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err =
                    SolPegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
                        btcTxHash: FixedBytes::<32>::default(),
                    });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InternalError);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_stream_not_found_by_denomination_smart_contract_raised()
     {
        let mut mock_instance = MockPegManagerContractApi::new();

        // just to make it clear that is invalid, but we do not care about the value as we force the SC to error
        let invalid_value = 2;

        let input = PegInAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: invalid_value,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_peg_in_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(invalid_value),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = SolPegManagerErrors::StreamNotFoundByDenomination(
                    StreamNotFoundByDenomination {
                        denomination: alloy_primitives::Uint::from(invalid_value),
                    },
                );
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            PegManagerErrors::StreamNotFoundByDenomination
        );
    }

    #[allow(unused)]
    fn init_logger() {
        env_logger::builder().is_test(true).try_init();
    }

    fn generate_expected_error<T: SolInterface>(test: T) -> ErrorPayload {
        let encoded_error = format!("0x{}", hex::encode(test.abi_encode()));
        let error = ERROR_TEMPLATE.replace("<to_replace>", &encoded_error);
        serde_json::from_str::<ErrorPayload>(&error).unwrap()
    }
}
