use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::PegManagerErrors;
use crate::types::{PegInAddressInput, PegInAddressOutput};
use alloy_primitives::{Address, FixedBytes};
use log::info;

// TODO(Jira): generate Try_From for the input struct like in the other cases - https://rsklabs.atlassian.net/browse/UB-108

pub(crate) struct GetTemporaryPegInAddressCall<C: PegManagerContractApi> {
    contract: C,
}

impl<C: PegManagerContractApi> GetTemporaryPegInAddressCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetTemporaryPegInAddressCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors> {
        info!("Init GetTemporaryPegInAddress for: {:?}", input);

        let rootstock_deposit_address: Address = input
            .rootstock_deposit_address
            .parse::<Address>()
            .map_err(|e| {
                PegManagerErrors::InvalidAddress(format!(
                    "Failed to parse rootstock_deposit_address: {}",
                    e
                ))
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> = input
            .btc_reimbursement_pub_key
            .parse::<FixedBytes<32>>()
            .map_err(|e| {
                PegManagerErrors::InvalidPublicKey(format!(
                    "Failed to parse btc_reimbursement_pub_key: {}",
                    e
                ))
            })?;

        let receipt = self
            .contract
            .get_temporary_peg_in_address_call(
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
            )
            .await?;

        info!(
            "GetTemporaryPegInAddress successful, deposit address: {}",
            receipt.bitcoinDepositAddress
        );

        Ok(PegInAddressOutput {
            address: receipt.bitcoinDepositAddress.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        InvalidAddress, InvalidPublicKey, SolBitcoinManagerErrors,
    };
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::interactions::get_temporary_peg_in_address::{
        GetTemporaryPegInAddressCall, PegInAddressInput,
    };
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use crate::contracts::peg_manager::SolPegManager::getTemporaryPegInAddressReturn;
    use crate::rsk_gateway::PegManagerErrors;
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::RpcError::ErrorResp;
    use alloy_primitives::Address;
    use alloy_primitives::FixedBytes;
    use mockall::predicate::{always, eq};

    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;

    #[cfg(test)]
    impl GetTemporaryPegInAddressCall<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            GetTemporaryPegInAddressCall { contract }
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
        matches!(result.err().unwrap(), PegManagerErrors::InvalidAddress(_));
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
                let expected_err = SolBitcoinManagerErrors::InvalidAddress(InvalidAddress {
                    _address: Address::default(),
                });
                let expected_err_payload = generate_contract_revert_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let interaction = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), PegManagerErrors::InvalidAddress(_));
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
        matches!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey(_));
    }

    // there are more errors that could be raised by the smart contract, but those are tested either on peg_manager.rs or bitcoin_manager.rs
    #[tokio::test]
    async fn test_get_temporary_pegin_address_revert() {
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
                let expected_err = SolBitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
                    publicKey: FixedBytes::<32>::default(),
                });
                let expected_err_payload = generate_contract_revert_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let call = GetTemporaryPegInAddressCall::new_for_tests(mock_instance);

        let result = call.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey(_));
    }

    #[allow(unused)]
    fn init_logger() {
        env_logger::builder().is_test(true).try_init();
    }
}
