use crate::contracts::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
use crate::contracts::peg_manager::PegManagerAlloy::{
    BtcTransaction, PegInRequestTxSPVProof, PegManagerAlloyErrors, PegManagerAlloyInstance,
    getTemporaryPegInAddressReturn, registerPegInRequestReturn,
};
use alloy_contract::Error::TransportError;
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, FixedBytes, U256};
use alloy_provider::RootProvider;
use alloy_sol_types::{SolInterface, sol};
use anyhow::Result;
use log::{debug, error, info};
use thiserror::Error;

use crate::contracts::bitcoin_manager::{BitcoinTransaction, ParseFieldError};
#[cfg(feature = "testing")]
use mockall::automock;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "testing", automock)]
pub trait PegManagerInstance {
    #[allow(non_snake_case)]
    #[allow(async_fn_in_trait)]
    async fn getTemporaryPegInAddress(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;

    #[allow(non_snake_case)]
    #[allow(async_fn_in_trait)]
    async fn registerPeginRequest(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn>;
}

#[derive(Serialize, Deserialize, Debug)]
// TODO(iago) use alloy type, otherwise this will grow a lot (and this way it automatically reacts to abi changes)
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
// TODO(iago) use alloy type, otherwise this will grow a lot (and this way it automatically reacts to abi changes)
pub struct PeginAddressOutput {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RegisterPeginInput {
    pub block_hash: String,
    pub btc_tx: BitcoinTransaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

impl TryFrom<RegisterPeginInput> for PegInRequestTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: RegisterPeginInput) -> Result<Self, Self::Error> {
        let block_hash =
            FixedBytes::<32>::from_hex(&value.block_hash).map_err(ParseFieldError::ParseHex)?;

        let btc_tx: BtcTransaction = value.btc_tx.try_into()?;

        let merkle_branches_hashes = value
            .merkle_branch_hashes
            .into_iter()
            .map(|hash| {
                hash.parse::<FixedBytes<32>>()
                    .map_err(ParseFieldError::ParseHex)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let merkle_branch_path = U256::from_str_radix(&value.merkle_branch_path, 16)
            .map_err(ParseFieldError::ParseNum)?;

        Ok(PegInRequestTxSPVProof {
            blockHash: block_hash,
            btcTx: btc_tx,
            merkleBranchPath: merkle_branch_path,
            merkleBranchHashes: merkle_branches_hashes,
        })
    }
}

sol!(
    #[sol(rpc)]
    PegManagerAlloy,
    "../config/dev/abi/PegManager.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);

// needed so we can create a PegManagerApi trait for tests mocking
pub struct PegManagerAlloyWrapper {
    inner: PegManagerAlloyInstance<(), RootProvider>,
}

impl PegManagerInstance for PegManagerAlloyWrapper {
    #[allow(non_snake_case)]
    async fn getTemporaryPegInAddress(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn> {
        self.inner
            .getTemporaryPegInAddress(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .call()
            .await
    }

    #[allow(non_snake_case)]
    async fn registerPeginRequest(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn> {
        self.inner.registerPegInRequest(input).call().await
    }
}

pub struct PegManager<I>
where
    I: PegManagerInstance,
{
    address: Address,
    instance: I,
}

impl PegManager<PegManagerAlloyWrapper> {
    pub fn init(provider: &RootProvider, address: Address) -> Result<Self> {
        let instance = PegManagerAlloy::new(address, provider.clone());

        Ok(PegManager {
            address,
            instance: PegManagerAlloyWrapper { inner: instance },
        })
    }
}

impl<I> PegManager<I>
where
    I: PegManagerInstance,
{
    pub(crate) async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerErrors> {
        info!("Interacting with PegManager @ {}", self.address);
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
            .instance
            .getTemporaryPegInAddress(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .await;

        match result {
            Ok(data) => {
                debug!(
                    "Bitcoin Deposit Address for {:?}: {}",
                    input, data.bitcoinDepositAddress
                );

                Ok(PeginAddressOutput {
                    address: data.bitcoinDepositAddress.to_string(),
                })
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => Err(Self::decode_contract_error(e)),
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

    pub(crate) async fn register_peg_in_request(
        &self,
        input: RegisterPeginInput,
    ) -> Result<(), PegManagerErrors> {
        let parsed_input = match PegInRequestTxSPVProof::try_from(input) {
            Ok(i) => i,
            Err(e) => {
                // TODO(iago) distinguish error types
                error!("Failed to parse RegisterPeginInput: {}", e);
                return Err(PegManagerErrors::InternalError);
            }
        };

        let result = self.instance.registerPeginRequest(parsed_input).await;
        match result {
            Ok(_) => {
                info!("PegInRequestTxSPVProof registered");
                return Ok(());
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => {
                    error!("Error calling PegManager: {:?}", e);
                }
                None => {
                    // TODO(iago) decode_contract_error, etc.
                    error!("Missing ErrorPayload in PegManager error {:?}", err);
                }
            },
            Err(e) => {
                error!("Error calling PegManager: {:?}", e);
            }
        }

        // TODO(iago) properly handle
        return Err(PegManagerErrors::InternalError);
    }

    fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
        let revert_data = if let Some(data) = error_payload.as_revert_data() {
            data
        } else {
            error!("No revert data found in PegManager error {error_payload}");
            return PegManagerErrors::InternalError;
        };

        let decoded_error = PegManagerAlloyErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            let decoded_error = decoded_error.unwrap();
            return match decoded_error {
                PegManagerAlloyErrors::StreamNotFoundByDenomination(e) => {
                    error!("StreamNotFoundByDenomination {}", e.denomination);
                    PegManagerErrors::StreamNotFoundByDenomination
                }
                _ => {
                    // TODO properly handle other errors when the related flow is implemented
                    error!("Unhandled error: {:?}", error_payload);
                    PegManagerErrors::InternalError
                }
            };
        }

        let decoded_error = BitcoinManagerErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            return match decoded_error.unwrap() {
                BitcoinManagerErrors::InvalidAddress(e) => {
                    error!("InvalidAddress {}", e._address);
                    PegManagerErrors::InvalidAddress
                }
                BitcoinManagerErrors::InvalidPublicKey(e) => {
                    error!("InvalidPublicKey {}", e.publicKey);
                    PegManagerErrors::InvalidPublicKey
                }
                BitcoinManagerErrors::InvalidValue(e) => {
                    error!("InvalidValue {}", e._value);
                    PegManagerErrors::InvalidValue
                }
                _ => {
                    // TODO properly handle other errors when the related flow is implemented
                    error!("Unhandled error on BitcoinManager: {:?}", error_payload);
                    PegManagerErrors::InternalError
                }
            };
        }

        error!("Unknown error on BitcoinManager: {:?}", error_payload);
        PegManagerErrors::InternalError
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum PegManagerErrors {
    #[error("Internal Error")]
    InternalError,
    #[error("Stream not found by denomination")]
    StreamNotFoundByDenomination,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid address")]
    InvalidAddress,
    #[error("Invalid value")]
    InvalidValue,
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use crate::contracts::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, InvalidAddress, InvalidPublicKey,
    };
    use crate::contracts::peg_manager::PegManagerAlloy::{
        AlreadyRegisteredPegIn, PegManagerAlloyErrors, StreamNotFoundByDenomination,
        getTemporaryPegInAddressReturn,
    };
    use crate::contracts::peg_manager::{
        MockPegManagerInstance, PegManager, PegManagerErrors, PeginAddressInput,
    };
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::ErrorPayload;
    use alloy_json_rpc::RpcError::ErrorResp;
    use alloy_primitives::Address;
    use alloy_primitives::FixedBytes;
    use alloy_sol_types::SolInterface;
    use mockall::predicate::{always, eq};

    const CONTRACT_ADDRESS: &str = "0x8c86ead50dc378858163debca4b59b039943f05d";
    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;

    const ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };
        let expected_deposit_address = "0xfake0deposit0address".to_string();
        let output = getTemporaryPegInAddressReturn {
            bitcoinDepositAddress: expected_deposit_address.clone(),
        };

        mock_instance
            .expect_getTemporaryPegInAddress()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| Ok(output.clone()))
            .times(1);

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().address, expected_deposit_address);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_preliminary_validation() {
        let mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_smart_contract_raised() {
        let mut mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            // it has to be valid here in order to pass the preliminary validation (non SC)
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_getTemporaryPegInAddress()
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

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_preliminary_validation() {
        let mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: "0xinvalid_pub_key".to_string(),
        };

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_smart_contract_raised() {
        let mut mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_getTemporaryPegInAddress()
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

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_internal_server_error() {
        let mut mock_instance = MockPegManagerInstance::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_getTemporaryPegInAddress()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err =
                    PegManagerAlloyErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
                        btcTxHash: FixedBytes::<32>::default(),
                    });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InternalError);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_stream_not_found_by_denomination_smart_contract_raised()
     {
        let mut mock_instance = MockPegManagerInstance::new();

        // just to make it clear that is invalid, but we do not care about the value as we force the SC to error
        let invalid_value = 2;

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: invalid_value,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_getTemporaryPegInAddress()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(invalid_value),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = PegManagerAlloyErrors::StreamNotFoundByDenomination(
                    StreamNotFoundByDenomination {
                        denomination: alloy_primitives::Uint::from(invalid_value),
                    },
                );
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

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
