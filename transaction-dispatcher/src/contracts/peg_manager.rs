use crate::contracts::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
use crate::contracts::peg_manager::PegManager::{PegManagerErrors, PegManagerInstance};
use crate::types::{BaseContract, PeginAddressInput, PeginAddressOutput};
use alloy_contract::Error::TransportError;
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::{Address, FixedBytes};
use alloy_provider::RootProvider;
use alloy_sol_types::{SolInterface, sol};
use anyhow::Result;
use log::{debug, error, info};
use thiserror::Error;

sol!(
    #[sol(rpc)]
    PegManager,
    "../config/dev/abi/PegManager.json"
);

pub struct PegManagerContract {
    address: Address,
    instance: PegManagerInstance<(), RootProvider>,
}

impl BaseContract for PegManagerContract {
    fn init(provider: &RootProvider, address: Address) -> Result<Self> {
        let contract_instance = PegManager::new(address, provider.clone());

        Ok(PegManagerContract {
            address,
            instance: contract_instance,
        })
    }

    fn contract_name() -> String {
        "PegManager".to_string()
    }
}

impl PegManagerContract {
    pub(crate) async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerContractErrors> {
        info!("Interacting with PegManager @ {}", self.address);

        let rootstock_deposit_address: Address = input
            .rootstock_deposit_address
            .parse::<Address>()
            .map_err(|e| {
                error!("Failed to parse rootstock_deposit_address: {}", e);
                PegManagerContractErrors::InvalidAddress
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> = input
            .btc_reimbursement_pub_key
            .parse::<FixedBytes<32>>()
            .map_err(|e| {
                error!("Failed to parse btc_reimbursement_pub_key: {}", e);
                PegManagerContractErrors::InvalidPublicKey
            })?;

        let tmp_address_call = self.instance.getTemporaryPegInAddress(
            rootstock_deposit_address,
            value,
            btc_reimbursement_pub_key,
        );

        let result = tmp_address_call.call().await;
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
                    Err(PegManagerContractErrors::InternalError)
                }
            },
            Err(e) => {
                error!("Error calling PegManager: {:?}", e);
                Err(PegManagerContractErrors::InternalError)
            }
        }
    }
}

impl PegManagerContract {
    fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerContractErrors {
        let revert_data = error_payload.as_revert_data();
        if revert_data.is_none() {
            error!("No revert data found in PegManager error {error_payload}");
            return PegManagerContractErrors::InternalError;
        }

        let revert_data = revert_data.unwrap();

        let decoded_error = PegManagerErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            let decoded_error = decoded_error.unwrap();
            return match decoded_error {
                PegManagerErrors::StreamNotFoundByDenomination(e) => {
                    error!("StreamNotFoundByDenomination {}", e.denomination);
                    PegManagerContractErrors::StreamNotFoundByDenomination
                }
                _ => {
                    // TODO properly handle other errors when the related flow is implemented
                    error!("Unhandled error: {:?}", error_payload);
                    PegManagerContractErrors::InternalError
                }
            };
        }

        let decoded_error = BitcoinManagerErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            return match decoded_error.unwrap() {
                BitcoinManagerErrors::InvalidAddress(e) => {
                    error!("InvalidAddress {}", e._address);
                    PegManagerContractErrors::InvalidAddress
                }
                BitcoinManagerErrors::InvalidPublicKey(e) => {
                    error!("InvalidPublicKey {}", e.publicKey);
                    PegManagerContractErrors::InvalidPublicKey
                }
                BitcoinManagerErrors::InvalidValue(e) => {
                    error!("InvalidValue {}", e._value);
                    PegManagerContractErrors::InvalidValue
                }
                _ => {
                    // TODO properly handle other errors when the related flow is implemented
                    error!("Unhandled error on BitcoinManager: {:?}", error_payload);
                    PegManagerContractErrors::InternalError
                }
            };
        }

        error!("Unknown error on BitcoinManager: {:?}", error_payload);
        PegManagerContractErrors::InternalError
    }
}

#[derive(Debug, Error)]
pub enum PegManagerContractErrors {
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
