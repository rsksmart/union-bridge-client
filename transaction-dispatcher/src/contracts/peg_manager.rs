use crate::contracts::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
use crate::contracts::peg_manager::PegManagerAlloy::{
    PegManagerAlloyErrors, PegManagerAlloyInstance, getTemporaryPegInAddressReturn,
};
use crate::types::{PeginAddressInput, PeginAddressOutput};
use alloy_contract::Error::TransportError;
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::{Address, FixedBytes};
use alloy_provider::RootProvider;
use alloy_sol_types::{SolInterface, sol};
use anyhow::Result;
use log::{debug, error, info};
use thiserror::Error;

#[cfg(feature = "generate-mocks")]
use mockall::automock;

sol!(
    #[sol(rpc)]
    PegManagerAlloy,
    "../config/dev/abi/PegManager.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);

#[cfg_attr(feature = "generate-mocks", automock)]
pub trait PegManagerInstance {
    #[allow(non_snake_case)]
    async fn getTemporaryPegInAddress(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;
}

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

    fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
        let revert_data = error_payload.as_revert_data();
        if revert_data.is_none() {
            error!("No revert data found in PegManager error {error_payload}");
            return PegManagerErrors::InternalError;
        }

        let revert_data = revert_data.unwrap();

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

#[cfg(all(test, feature = "generate-mocks"))]
mod tests {
    use super::*;
    use mockall::predicate::eq;

    const CONTRACT_ADDRESS: &str = "0x8c86ead50dc378858163debca4b59b039943f05d";
    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_instance = MockPegManagerInstance::new();
        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: 1000,
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
                eq(1000),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| Ok(output.clone()));

        let peg_manager = PegManager {
            address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().address, expected_deposit_address);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address() {
        let mock_instance = MockPegManagerInstance::new();
        let input = PeginAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: 1000,
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
    async fn test_get_temporary_pegin_address_invalid_public_key() {
        let mock_instance = MockPegManagerInstance::new();
        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: 1000,
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

    // #[tokio::test]
    // async fn test_get_temporary_pegin_address_stream_not_found_by_denomination() {
    //     let mut mock_instance = MockPegManagerInstance::new();
    //     let input = PeginAddressInput {
    //         rootstock_deposit_address: VALID_ADDRESS.to_string(),
    //         value: 1000,
    //         btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
    //     };
    //
    //     mock_instance
    //         .expect_getTemporaryPegInAddress()
    //         .with(
    //             eq(VALID_ADDRESS.parse::<Address>().unwrap()),
    //             eq(1000),
    //             eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
    //         )
    //         .returning(|_, _, _| {
    //             Err(alloy_contract::Error::TransportError(
    //                 PegManagerAlloyErrors::StreamNotFoundByDenomination {
    //                     denomination: "denomination".to_string(),
    //                 }
    //             ))
    //         });
    //
    //     let peg_manager = PegManager {
    //         address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
    //         instance: mock_instance,
    //     };
    //
    //     let result = peg_manager.get_temporary_pegin_address(input).await;
    //     assert!(result.is_err());
    //     assert_eq!(
    //         result.err().unwrap(),
    //         PegManagerErrors::StreamNotFoundByDenomination
    //     );
    // }
}
