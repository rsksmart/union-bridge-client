use crate::{
    contracts::peg_manager::PegManagerContractApi,
    rsk_gateway::DomainErrors,
    types::{PeginAddressInput, PeginAddressOutput},
};
use alloy_primitives::{Address, FixedBytes};
use log::info;

// TODO(Jira): generate Try_From for the input struct like in the other cases - https://rsklabs.atlassian.net/browse/UB-108

#[derive(Clone)]
pub(crate) struct GetTemporaryPeginAddressCall<C: PegManagerContractApi> {
    contract: C,
}

impl<C: PegManagerContractApi> GetTemporaryPeginAddressCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetTemporaryPeginAddressCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors> {
        info!("Init GetTemporaryPeginAddressCall for: {input:?}");

        let rootstock_deposit_address: Address = input
            .rootstock_deposit_address
            .parse::<Address>()
            .map_err(|e| {
                DomainErrors::InvalidAddress(format!(
                    "Failed to parse rootstock_deposit_address: {e}"
                ))
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> = input
            .btc_reimbursement_pub_key
            .parse::<FixedBytes<32>>()
            .map_err(|e| {
                DomainErrors::InvalidCompressedPubKey(format!(
                    "Failed to parse btc_reimbursement_pub_key: {e}"
                ))
            })?;

        let address = self
            .contract
            .call_get_temporary_pegin_address(
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
            )
            .await?;

        info!("GetTemporaryPeginAddress successful, deposit address: {address:?}");

        Ok(PeginAddressOutput { address })
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::interactions::get_temporary_pegin_address::{
        GetTemporaryPeginAddressCall, PeginAddressInput,
    };
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use crate::rsk_gateway::DomainErrors;
    use alloy_primitives::Address;
    use alloy_primitives::FixedBytes;
    use mockall::predicate::always;
    use mockall::predicate::eq;
    use union_contracts::bindings::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, InvalidAddress, InvalidPublicKey,
    };
    use union_contracts::bindings::peg_manager::PegManager::getTemporaryPeginAddressReturn;

    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;

    impl GetTemporaryPeginAddressCall<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            GetTemporaryPeginAddressCall { contract }
        }
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };
        let expected_deposit_address = "0xfake0deposit0address".to_string();
        let output = getTemporaryPeginAddressReturn {
            bitcoinDepositAddress: expected_deposit_address.clone(),
        };

        mock_instance
            .expect_call_get_temporary_pegin_address()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| Ok(output.bitcoinDepositAddress.clone()))
            .times(1);

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().address, expected_deposit_address);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_preliminary_validation() {
        let mock_instance = MockPegManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidAddress(_));
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_smart_contract_raised() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PeginAddressInput {
            // it has to be valid here in order to pass the preliminary validation (non SC)
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_call_get_temporary_pegin_address()
            .with(
                always(),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidAddress(InvalidAddress {
                    _address: Address::default(),
                });
                Err(generate_contract_revert_error(&expected_err))
            })
            .times(1);

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidAddress(_));
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_preliminary_validation() {
        let mock_instance = MockPegManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: "0xinvalid_pub_key".to_string(),
        };

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_instance);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::InvalidCompressedPubKey(_)
        );
    }

    // there are more errors that could be raised by the smart contract, but those are tested either on peg_manager.rs or bitcoin_manager.rs
    #[tokio::test]
    async fn test_get_temporary_pegin_address_revert() {
        let mut mock_instance = MockPegManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_call_get_temporary_pegin_address()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
                    publicKey: FixedBytes::<32>::default(),
                });
                Err(generate_contract_revert_error(&expected_err))
            })
            .times(1);

        let call = GetTemporaryPeginAddressCall::new_for_tests(mock_instance);

        let result = call.run(input).await;
        assert!(result.is_err());
        matches!(
            result.err().unwrap(),
            DomainErrors::InvalidCompressedPubKey(_)
        );
    }

    #[allow(unused)]
    fn init_logger() {
        env_logger::builder().is_test(true).try_init();
    }
}
