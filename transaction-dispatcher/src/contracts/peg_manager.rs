use crate::contracts::bitcoin_manager;
use crate::contracts::common::send_tx_with_gas_bump;
use crate::contracts::peg_manager::SolPegManager::{
    BtcTransaction, PegInRequestTxSPVProof, SolPegManagerErrors, SolPegManagerInstance,
    getTemporaryPegInAddressReturn,
};
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::{SolInterface, sol};
use anyhow::Result;
use log::error;

use crate::contracts::bitcoin_manager::ParseFieldError;

use crate::rsk_gateway::PegManagerErrors;
#[cfg(test)]
use mockall::automock;

// re-export for convenience
pub(crate) use crate::contracts::interactions::get_temporary_peg_in_address;
pub(crate) use crate::contracts::interactions::register_peg_in_request;
use crate::types::RegisterPegInInput;

sol!(
    #[sol(rpc)]
    SolPegManager,
    "../config/local/abi/PegManager.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);

#[cfg_attr(test, automock)]
pub trait PegManagerContractApi {
    #[allow(async_fn_in_trait)]
    async fn get_temporary_peg_in_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;

    #[allow(async_fn_in_trait)]
    async fn register_peg_in_request_send(
        &self,
        input: PegInRequestTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;
}

// needed so we can create a PegManagerContractApi trait for tests mocking
#[derive(Clone)]
pub struct PegManagerContract<P: Provider> {
    contract_instance: SolPegManagerInstance<(), P>,
}

impl<P: Provider> PegManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        let contract_instance = SolPegManager::new(contract_address, provider);
        PegManagerContract { contract_instance }
    }
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

    async fn register_peg_in_request_send(
        &self,
        input: PegInRequestTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            || self.contract_instance.registerPegInRequest(input.clone()),
            gas_bumps,
        )
        .await
    }
}

pub(crate) fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
    let revert_data = if let Some(data) = error_payload.as_revert_data() {
        data
    } else {
        return PegManagerErrors::NoRevertError(format!(
            "Not a PegManagerError: {:?}",
            error_payload
        ));
    };

    if let Some(err) = decode_self_error(&revert_data) {
        return err;
    }

    if let Some(err) = bitcoin_manager::decode_contract_error(error_payload) {
        return err;
    }

    PegManagerErrors::UnknownContractError(format!("Unknown PegManagerError: {:?}", error_payload))
}

fn decode_self_error(revert_data: &Bytes) -> Option<PegManagerErrors> {
    let decoded_error = SolPegManagerErrors::abi_decode(&revert_data, true);
    if decoded_error.is_ok() {
        let decoded_error = decoded_error.unwrap();
        // TODO(create-Jira) - review all errors and conceptually merge into or create new PegManagerErrors
        return Some(match decoded_error {
            SolPegManagerErrors::AddressEmptyCode(e) => PegManagerErrors::UnhandledContractError(
                format!("SolPegManagerErrors#AddressEmptyCode {}", e.target),
            ),
            SolPegManagerErrors::AlreadyRegisteredPegIn(e) => {
                PegManagerErrors::AlreadyRegisteredPegIn(format!(
                    "SolPegManagerErrors#AlreadyRegisteredPegIn {}",
                    e.btcTxHash
                ))
            }
            SolPegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcBlockNotInBestChain {}",
                    e.blockHash
                ))
            }
            SolPegManagerErrors::BridgeBtcBlockTooOld(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcBlockTooOld {}",
                    e.maxDepth
                ))
            }
            SolPegManagerErrors::BridgeBtcInconsistentBlock(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcInconsistentBlock {}",
                    e.blockHash
                ))
            }
            SolPegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcInexistantBlockHash {}",
                    e.blockHash
                ))
            }
            SolPegManagerErrors::BridgeBtcTxInvalidMerkleBranch(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcTxInvalidMerkleBranch {} - {} - {:?}",
                    e.txHash, e.merkleBranchPath, e.merkleBranchHashes
                ))
            }
            SolPegManagerErrors::BridgeBtcUnknownError(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#BridgeBtcUnknownError {}",
                    e.errorCode
                ))
            }
            SolPegManagerErrors::ERC1967InvalidImplementation(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#ERC1967InvalidImplementation {}",
                    e.implementation
                ))
            }
            SolPegManagerErrors::ERC1967NonPayable(_) => PegManagerErrors::UnhandledContractError(
                "SolPegManagerErrors#ERC1967NonPayable".to_string(),
            ),
            SolPegManagerErrors::FailedCall(_) => PegManagerErrors::UnhandledContractError(
                "SolPegManagerErrors#FailedCall".to_string(),
            ),
            SolPegManagerErrors::InvalidInitialization(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolPegManagerErrors#InvalidInitialization".to_string(),
                )
            }
            SolPegManagerErrors::NoEmptySlot(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#NoEmptySlot {} - {}",
                    e.packetNumber, e.streamId
                ))
            }
            SolPegManagerErrors::NotEnoughConfirmations(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#NotEnoughConfirmations {} - {}",
                    e.expected, e.actual
                ))
            }
            SolPegManagerErrors::NotInitializing(_) => PegManagerErrors::UnhandledContractError(
                "SolPegManagerErrors#NotInitializing".to_string(),
            ),
            SolPegManagerErrors::OwnableInvalidOwner(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#OwnableInvalidOwner {}",
                    e.owner
                ))
            }
            SolPegManagerErrors::OwnableUnauthorizedAccount(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#OwnableUnauthorizedAccount {}",
                    e.account
                ))
            }
            SolPegManagerErrors::PacketOutOfBound(e) => PegManagerErrors::UnhandledContractError(
                format!("SolPegManagerErrors#PacketOutOfBound {}", e.packetNumber),
            ),
            SolPegManagerErrors::StreamNotFoundByDenomination(e) => {
                PegManagerErrors::StreamNotFoundByDenomination(format!(
                    "SolPegManagerErrors#StreamNotFoundByDenomination {}",
                    e.denomination
                ))
            }
            SolPegManagerErrors::tooManyDenominations(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolPegManagerErrors#tooManyDenominations {}",
                    e.maxDenominationsSize
                ))
            }
            SolPegManagerErrors::UUPSUnauthorizedCallContext(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolPegManagerErrors#UUPSUnauthorizedCallContext".to_string(),
                )
            }
            SolPegManagerErrors::UUPSUnsupportedProxiableUUID(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolPegManagerErrors#UUPSUnsupportedProxiableUUID".to_string(),
                )
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
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::peg_manager::SolPegManager::{
        AlreadyRegisteredPegIn, NotInitializing, SolPegManagerErrors, StreamNotFoundByDenomination,
    };
    use crate::contracts::peg_manager::decode_contract_error;
    use crate::rsk_gateway::PegManagerErrors;

    #[test]
    fn test_already_registered_peg_in() {
        let expected_err = SolPegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
            btcTxHash: "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                .parse()
                .expect("Failed to parse tx hash"),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::AlreadyRegisteredPegIn(_));
    }

    #[test]
    fn test_stream_not_found_by_denomination() {
        let expected_err =
            SolPegManagerErrors::StreamNotFoundByDenomination(StreamNotFoundByDenomination {
                denomination: alloy_primitives::Uint::from(125),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::StreamNotFoundByDenomination(_));
    }

    // check one of the errors to ensure the code keeps covering also SolBitcoinManagerErrors
    // but the tests for SolBitcoinManagerErrors are in the bitcoin_manager.rs
    #[test]
    fn test_bitcoin_manager_error() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputNumber(IncorrectOutputNumber {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidPegInRequestData(_));
    }

    // check one of the errors to ensure the mapping to InternalError keeps working
    // there are more errors that map to InternalError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = SolPegManagerErrors::NotInitializing(NotInitializing {});

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::UnhandledContractError(_));
    }
}
