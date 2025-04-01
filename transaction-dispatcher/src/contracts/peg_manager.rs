use crate::contracts::bitcoin_manager;
use crate::contracts::common::{ContractInvokeReceipt, send_with_gas};
use crate::contracts::peg_manager::SolPegManager::{
    BtcTransaction, PegInRequestTxSPVProof, SolPegManagerErrors, SolPegManagerInstance,
    getTemporaryPegInAddressReturn, registerPegInRequestReturn,
};
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_provider::Provider;
use alloy_provider::network::EthereumWallet;
use alloy_sol_types::{SolInterface, sol};
use anyhow::{Context, Result, bail};
use log::{debug, error, info, warn};
use std::sync::Arc;
use thiserror::Error;

use crate::contracts::bitcoin_manager::ParseFieldError;
use crate::use_cases::get_temporary_peg_in_address::{
    GetTemporaryPegInAddressCall, PegInAddressInput, PegInAddressOutput,
};
use crate::use_cases::register_peg_in_request::{RegisterPegInInput, RegisterPegInRequestInvoke};
#[cfg(feature = "testing")]
use mockall::automock;

sol!(
    #[sol(rpc)]
    SolPegManager,
    "../config/dev/abi/PegManager.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);

#[cfg_attr(feature = "testing", automock)]
pub trait PegManagerContractApi {
    #[allow(async_fn_in_trait)]
    async fn get_temporary_peg_in_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;

    #[allow(async_fn_in_trait)]
    async fn register_peg_in_request_call(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn>;

    #[allow(async_fn_in_trait)]
    async fn register_peg_in_request_send(
        &self,
        signer: &EthereumWallet,
        input: PegInRequestTxSPVProof,
    ) -> Result<ContractInvokeReceipt>;
}

// needed so we can create a PegManagerContractApi trait for tests mocking
pub struct PegManagerContract<P: Provider> {
    contract_instance: SolPegManagerInstance<(), P>,
}

impl<P: Provider> PegManagerContractApi for PegManagerContract<P> {
    async fn get_temporary_peg_in_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn> {
        self.contract_instance
            .getTemporaryPegInAddress(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .call()
            .await
    }

    async fn register_peg_in_request_call(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn> {
        self.contract_instance
            .registerPegInRequest(input)
            .call()
            .await
    }

    async fn register_peg_in_request_send(
        &self,
        signer: &EthereumWallet,
        input: PegInRequestTxSPVProof,
    ) -> Result<ContractInvokeReceipt> {
        // TODO(iago) move chain_id and nonce retrieval to a common place

        let chain_id = self
            .contract_instance
            .provider()
            .get_chain_id()
            .await
            .context("getting chain id")?;

        let nonce = self
            .contract_instance
            .provider()
            .get_transaction_count(signer.default_signer().address())
            .await
            .context("getting nonce")?;

        let tx_builder = self
            .contract_instance
            .registerPegInRequest(input)
            .chain_id(chain_id)
            .nonce(nonce);

        let mut estimated_gas = tx_builder.estimate_gas().await.context("estimating gas")?;

        // TODO(iago) make the retries configurable
        for _ in 0..3 {
            let receipt = send_with_gas(
                self.contract_instance.provider(),
                tx_builder.clone(),
                estimated_gas,
            )
            .await
            .context("getting receipt")?;

            debug!("Transaction receipt: {:?}", receipt);

            if receipt.status {
                return Ok(receipt);
            } else if receipt.gas_used >= estimated_gas {
                warn!("Bumping transaction gas");
                estimated_gas = (estimated_gas as f64 * 1.1) as u64; // TODO(iago) 1.1 configurable
            }
        }

        bail!("Failed to call registerPegInRequest")
    }
}

pub trait PegManagerGatewayApi {
    async fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors>;

    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<ContractInvokeReceipt, PegManagerErrors>;
}

pub struct PegManagerGateway<C: PegManagerContractApi> {
    contract_address: Address,
    get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall<C>,
    register_peg_in_request_invoke: RegisterPegInRequestInvoke<C>,
}

impl<P: Provider> PegManagerGateway<PegManagerContract<P>> {
    pub fn init(provider: P, signer: EthereumWallet, contract_address: Address) -> Result<Self> {
        let contract_instance = SolPegManager::new(contract_address, provider);
        let contract_wrapper = Arc::new(PegManagerContract { contract_instance });

        Ok(PegManagerGateway {
            contract_address,
            get_temporary_peg_in_address_call: GetTemporaryPegInAddressCall::new(
                contract_wrapper.clone(),
            ),
            register_peg_in_request_invoke: RegisterPegInRequestInvoke::new(
                contract_wrapper,
                signer,
            ),
        })
    }
}

impl<C: PegManagerContractApi> PegManagerGatewayApi for PegManagerGateway<C> {
    async fn get_temporary_peg_in_address(
        &self,
        input: PegInAddressInput,
    ) -> Result<PegInAddressOutput, PegManagerErrors> {
        info!(
            "Interacting with PegManager#getTemporaryPegInAddress @ {}",
            self.contract_address
        );

        self.get_temporary_peg_in_address_call.run(input).await
    }

    async fn register_peg_in_request(
        &self,
        input: RegisterPegInInput,
    ) -> Result<ContractInvokeReceipt, PegManagerErrors> {
        info!(
            "Interacting with PegManager#registerPegInRequest @ {}",
            self.contract_address
        );

        self.register_peg_in_request_invoke.run(input).await
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
    #[error("Already registered PegIn")]
    AlreadyRegisteredPegIn,
    #[error("Invalid value")]
    InvalidValue,
}

pub(crate) fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
    let revert_data = if let Some(data) = error_payload.as_revert_data() {
        data
    } else {
        error!("No revert data found in PegManager error {error_payload}");
        return PegManagerErrors::InternalError;
    };

    if let Some(err) = decode_self_error(&revert_data) {
        return err;
    }

    if let Some(err) = bitcoin_manager::decode_contract_error(&revert_data) {
        return err;
    }

    error!("Unknown error on PegManager: {:?}", error_payload);
    PegManagerErrors::InternalError
}

fn decode_self_error(revert_data: &Bytes) -> Option<PegManagerErrors> {
    let decoded_error = SolPegManagerErrors::abi_decode(&revert_data, true);
    if decoded_error.is_ok() {
        let decoded_error = decoded_error.unwrap();

        // TODO(iago) properly handle basic register_pegin errors instead of InternalError

        return Some(match decoded_error {
            SolPegManagerErrors::AddressEmptyCode(e) => {
                error!("SolPegManagerErrors#AddressEmptyCode {}", e.target);
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::AlreadyRegisteredPegIn(e) => {
                error!("SolPegManagerErrors#AlreadyRegisteredPegIn {}", e.btcTxHash);
                PegManagerErrors::AlreadyRegisteredPegIn
            }
            SolPegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
                error!(
                    "SolPegManagerErrors#BridgeBtcBlockNotInBestChain {}",
                    e.blockHash
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::BridgeBtcBlockTooOld(e) => {
                error!("SolPegManagerErrors#BridgeBtcBlockTooOld {}", e.maxDepth);
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::BridgeBtcInconsistentBlock(e) => {
                error!(
                    "SolPegManagerErrors#BridgeBtcInconsistentBlock {}",
                    e.blockHash
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
                error!(
                    "SolPegManagerErrors#BridgeBtcInexistantBlockHash {}",
                    e.blockHash
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::BridgeBtcTxInvalidMerkleBranch(e) => {
                error!(
                    "SolPegManagerErrors#BridgeBtcTxInvalidMerkleBranch {} - {} - {:?}",
                    e.txHash, e.merkleBranchPath, e.merkleBranchHashes
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::BridgeBtcUnknownError(e) => {
                error!("SolPegManagerErrors#BridgeBtcUnknownError {}", e.errorCode);
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::ERC1967InvalidImplementation(e) => {
                error!(
                    "SolPegManagerErrors#ERC1967InvalidImplementation {}",
                    e.implementation
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::ERC1967NonPayable(_) => {
                error!("SolPegManagerErrors#ERC1967NonPayable");
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::FailedCall(_) => {
                error!("SolPegManagerErrors#FailedCall");
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::InvalidInitialization(_) => {
                error!("SolPegManagerErrors#InvalidInitialization");
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::NoEmptySlot(e) => {
                error!(
                    "SolPegManagerErrors#NoEmptySlot {} - {}",
                    e.packetNumber, e.streamId
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::NotEnoughConfirmations(e) => {
                error!(
                    "SolPegManagerErrors#NotEnoughConfirmations {} - {}",
                    e.expected, e.actual
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::NotInitializing(_) => {
                error!("SolPegManagerErrors#NotInitializing");
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::OwnableInvalidOwner(e) => {
                error!("SolPegManagerErrors#OwnableInvalidOwner {}", e.owner);
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::OwnableUnauthorizedAccount(e) => {
                error!(
                    "SolPegManagerErrors#OwnableUnauthorizedAccount {}",
                    e.account
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::PacketOutOfBound(e) => {
                error!("SolPegManagerErrors#PacketOutOfBound {}", e.packetNumber);
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::StreamNotFoundByDenomination(e) => {
                error!(
                    "SolPegManagerErrors#StreamNotFoundByDenomination {}",
                    e.denomination
                );
                PegManagerErrors::StreamNotFoundByDenomination
            }
            SolPegManagerErrors::tooManyDenominations(e) => {
                error!(
                    "SolPegManagerErrors#tooManyDenominations {}",
                    e.maxDenominationsSize
                );
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::UUPSUnauthorizedCallContext(_) => {
                error!("SolPegManagerErrors#UUPSUnauthorizedCallContext");
                PegManagerErrors::InternalError
            }
            SolPegManagerErrors::UUPSUnsupportedProxiableUUID(_) => {
                error!("SolPegManagerErrors#UUPSUnsupportedProxiableUUID");
                PegManagerErrors::InternalError
            }
        });
    }
    None
}

impl TryFrom<RegisterPegInInput> for PegInRequestTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: RegisterPegInInput) -> Result<Self, Self::Error> {
        let block_hash =
            FixedBytes::<32>::from_hex(&value.block_hash).map_err(ParseFieldError::ParseHex)?;

        let btc_tx: BtcTransaction = value.btc_tx.try_into().map_err(|e| {
            error!("Failed to parse BTC transaction: {}", e);
            e
        })?;

        let merkle_branches_hashes = value
            .merkle_branch_hashes
            .into_iter()
            .map(|hash| {
                hash.parse::<FixedBytes<32>>()
                    .map_err(ParseFieldError::ParseHex)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                error!("Failed to convert merkle_branch_hashes: {:?}", e);
                e
            })?;

        let merkle_branch_path =
            U256::from_str_radix(&value.merkle_branch_path.trim_start_matches("0x"), 16).map_err(
                |e| {
                    error!("Failed to convert merkle_branch_path: {:?}", e);
                    e
                },
            )?;

        Ok(PegInRequestTxSPVProof {
            blockHash: block_hash,
            btcTx: btc_tx,
            merkleBranchPath: merkle_branch_path,
            merkleBranchHashes: merkle_branches_hashes,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        IncorrectOutputNumber, SolBitcoinManagerErrors,
    };
    use crate::contracts::common::tests::generate_contract_expected_error;
    use crate::contracts::peg_manager::SolPegManager::{
        AlreadyRegisteredPegIn, SolPegManagerErrors, StreamNotFoundByDenomination,
    };
    use crate::contracts::peg_manager::{PegManagerErrors, decode_contract_error};

    #[test]
    fn test_already_registered_peg_in() {
        let expected_err = SolPegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
            btcTxHash: "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                .parse()
                .expect("Failed to parse tx hash"),
        });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::AlreadyRegisteredPegIn);
    }

    #[test]
    fn test_stream_not_found_by_denomination() {
        let expected_err =
            SolPegManagerErrors::StreamNotFoundByDenomination(StreamNotFoundByDenomination {
                denomination: alloy_primitives::Uint::from(125),
            });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::StreamNotFoundByDenomination);
    }

    // check one of the errors to ensure the code keeps covering also SolBitcoinManagerErrors
    // but the tests for SolBitcoinManagerErrors are in the bitcoin_manager.rs
    #[test]
    fn test_bitcoin_manager_error() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputNumber(IncorrectOutputNumber {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InvalidPegInRequestData);
    }
}
