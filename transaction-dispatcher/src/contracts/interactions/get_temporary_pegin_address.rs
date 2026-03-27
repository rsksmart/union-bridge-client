use alloy_primitives::{Address, FixedBytes};
use log::debug;

use crate::contracts::pegin_manager::PeginManagerContractApi;
use crate::contracts::stream_manager::StreamManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{PeginAddressInput, PeginAddressOutput};

// TODO(Jira): generate Try_From for the input struct like in the other cases - UB-108

#[derive(Clone)]
pub(crate) struct GetTemporaryPeginAddressCall<
    C: PeginManagerContractApi,
    S: StreamManagerContractApi,
> {
    pegin_contract: C,
    stream_contract: S,
}

impl<C: PeginManagerContractApi, S: StreamManagerContractApi> GetTemporaryPeginAddressCall<C, S> {
    pub(crate) fn new(pegin_contract: C, stream_contract: S) -> Self {
        GetTemporaryPeginAddressCall { pegin_contract, stream_contract }
    }

    pub(crate) async fn run(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors> {
        debug!("Init GetTemporaryPeginAddressCall for: {input:?}");

        let rootstock_deposit_address: Address =
            input.rootstock_deposit_address.parse::<Address>().map_err(|e| {
                DomainErrors::InvalidAddress(format!(
                    "Failed to parse rootstock_deposit_address: {e}"
                ))
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> =
            input.btc_reimbursement_pub_key.parse::<FixedBytes<32>>().map_err(|e| {
                DomainErrors::InvalidCompressedPubKey(format!(
                    "Failed to parse btc_reimbursement_pub_key: {e}"
                ))
            })?;

        let pegin_data = self
            .pegin_contract
            .call_get_request_pegin_data(
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
            )
            .await?;

        debug!(
            "getRequestPeginData returned address={}, packetNumber={}",
            pegin_data.bitcoin_deposit_address, pegin_data.packet_number
        );

        let stream = self.stream_contract.call_get_stream(value).await?;

        let enabler_script = self
            .stream_contract
            .call_get_enabler_script_pubkey(stream.streamId, pegin_data.packet_number)
            .await?;

        let enabler_script_hex = format!("0x{}", hex::encode(&enabler_script));
        debug!("enablerScriptPubKey: {enabler_script_hex}");

        Ok(PeginAddressOutput {
            address: pegin_data.bitcoin_deposit_address,
            packet_number: pegin_data.packet_number,
            enabler_script_pubkey: enabler_script_hex,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, FixedBytes};
    use mockall::predicate::{always, eq};
    use union_contracts::bindings::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, InvalidAddress, InvalidPublicKey,
    };
    use union_contracts::bindings::stream_manager::StreamManager::{Stream, TimelockSettings};

    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::interactions::get_temporary_pegin_address::{
        GetTemporaryPeginAddressCall, PeginAddressInput,
    };
    use crate::contracts::pegin_manager::{MockPeginManagerContractApi, RequestPeginData};
    use crate::contracts::stream_manager::MockStreamManagerContractApi;
    use crate::rsk_gateway::DomainErrors;

    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;
    const FAKE_STREAM_ID: u64 = 1;
    const FAKE_PACKET_NUMBER: u64 = 0;

    impl GetTemporaryPeginAddressCall<MockPeginManagerContractApi, MockStreamManagerContractApi> {
        pub(crate) fn new_for_tests(
            pegin_contract: MockPeginManagerContractApi,
            stream_contract: MockStreamManagerContractApi,
        ) -> Self {
            GetTemporaryPeginAddressCall { pegin_contract, stream_contract }
        }
    }

    fn make_fake_stream(stream_id: u64) -> Stream {
        Stream {
            streamId: stream_id,
            denomination: VALID_VALUE,
            peginPacketPointer: FAKE_PACKET_NUMBER,
            peginConfirmations: 6,
            pegoutConfirmations: 6,
            timelockSettings: TimelockSettings::default(),
        }
    }

    fn setup_stream_mock(mock: &mut MockStreamManagerContractApi) {
        mock.expect_call_get_stream()
            .with(eq(VALID_VALUE))
            .returning(|_| Ok(make_fake_stream(FAKE_STREAM_ID)))
            .times(1);

        mock.expect_call_get_enabler_script_pubkey()
            .with(eq(FAKE_STREAM_ID), eq(FAKE_PACKET_NUMBER))
            .returning(|_, _| Ok(Bytes::from(vec![0x51, 0x20, 0xaa, 0xbb])))
            .times(1);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_pegin = MockPeginManagerContractApi::new();
        let mut mock_stream = MockStreamManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };
        let expected_deposit_address = "0xfake0deposit0address".to_string();

        mock_pegin
            .expect_call_get_request_pegin_data()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning({
                let addr = expected_deposit_address.clone();
                move |_, _, _| {
                    Ok(RequestPeginData {
                        bitcoin_deposit_address: addr.clone(),
                        packet_number: FAKE_PACKET_NUMBER,
                        _member_dispute_keys: vec![],
                        _available_slots: 5,
                    })
                }
            })
            .times(1);

        setup_stream_mock(&mut mock_stream);

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_pegin, mock_stream);

        let result = interaction.run(input).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.address, expected_deposit_address);
        assert_eq!(output.packet_number, FAKE_PACKET_NUMBER);
        assert_eq!(output.enabler_script_pubkey, "0x5120aabb");
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_preliminary_validation() {
        let mock_pegin = MockPeginManagerContractApi::new();
        let mock_stream = MockStreamManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_pegin, mock_stream);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidAddress(_));
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_smart_contract_raised() {
        let mut mock_pegin = MockPeginManagerContractApi::new();
        let mock_stream = MockStreamManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_pegin
            .expect_call_get_request_pegin_data()
            .with(always(), eq(VALID_VALUE), eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()))
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidAddress(InvalidAddress {
                    _address: Address::default(),
                });
                Err(generate_contract_revert_error(&expected_err))
            })
            .times(1);

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_pegin, mock_stream);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidAddress(_));
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_preliminary_validation() {
        let mock_pegin = MockPeginManagerContractApi::new();
        let mock_stream = MockStreamManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: "0xinvalid_pub_key".to_string(),
        };

        let interaction = GetTemporaryPeginAddressCall::new_for_tests(mock_pegin, mock_stream);

        let result = interaction.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidCompressedPubKey(_));
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_revert() {
        let mut mock_pegin = MockPeginManagerContractApi::new();
        let mock_stream = MockStreamManagerContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_pegin
            .expect_call_get_request_pegin_data()
            .with(eq(VALID_ADDRESS.parse::<Address>().unwrap()), eq(VALID_VALUE), always())
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
                    publicKey: FixedBytes::<32>::default(),
                });
                Err(generate_contract_revert_error(&expected_err))
            })
            .times(1);

        let call = GetTemporaryPeginAddressCall::new_for_tests(mock_pegin, mock_stream);

        let result = call.run(input).await;
        assert!(result.is_err());
        matches!(result.err().unwrap(), DomainErrors::InvalidCompressedPubKey(_));
    }

    #[allow(unused)]
    fn init_logger() {
        env_logger::builder().is_test(true).try_init();
    }
}
