use crate::contracts::common::send_tx_with_gas_bump;
use crate::contracts::peg_manager::SolPegManager::{
    BtcTransaction, BtcTxSPVProof, SolPegManagerErrors, SolPegManagerInstance,
    getTemporaryPegInAddressReturn,
};
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, FixedBytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::SolInterface;
use anyhow::Result;
use log::error;

use crate::contracts::bitcoin_manager::ParseFieldError;

// re-export for convenience
use crate::contracts::bitcoin_manager::SolBitcoinManager::SolBitcoinManagerErrors;
pub(crate) use crate::contracts::interactions::get_temporary_peg_in_address;
pub(crate) use crate::contracts::interactions::register_peg_in_request;
use crate::format_sol_err;
use crate::rsk_gateway::PegManagerErrors;
use crate::types::RegisterPegInInput;
use SolPegManagerErrors::*;
#[cfg(test)]
use mockall::automock;

include!(concat!(env!("OUT_DIR"), "/abi.rs"));

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
        input: BtcTxSPVProof,
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
        input: BtcTxSPVProof,
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

    if let Ok(peg_mgr_err) = SolPegManagerErrors::abi_decode(&revert_data, true) {
        return peg_mgr_err.into();
    }

    if let Ok(btc_mgr_err) = SolBitcoinManagerErrors::abi_decode(&revert_data, true) {
        return btc_mgr_err.into();
    }

    PegManagerErrors::UnknownContractError(format!("Unknown PegManagerError: {:?}", error_payload))
}

impl From<SolPegManagerErrors> for PegManagerErrors {
    fn from(err: SolPegManagerErrors) -> Self {
        match err {
            AlreadyRegisteredPegIn(e) => {
                PegManagerErrors::AlreadyRegisteredPegIn(format_sol_err!(e, e.btcTxHash))
            }
            AlreadyRegisteredPegInRequest(e) => {
                PegManagerErrors::AlreadyRegisteredPegInRequest(format_sol_err!(e, e.btcTxHash))
            }
            StreamNotFoundByDenomination(e) => {
                PegManagerErrors::StreamNotFoundByDenomination(format_sol_err!(e, e.denomination))
            }

            // all others default to Unhandled, but still log their fields
            AddressEmptyCode(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.target))
            }
            AlreadyRegisteredAcceptPegIn(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.btcTxHash))
            }
            BridgeAddressZero(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            BridgeBtcBlockNotInBestChain(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.blockHash))
            }
            BridgeBtcBlockTooOld(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.maxDepth))
            }
            BridgeBtcInconsistentBlock(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.blockHash))
            }
            BridgeBtcInexistantBlockHash(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.blockHash))
            }
            BridgeBtcTxInvalidMerkleBranch(e) => PegManagerErrors::UnhandledContractError(
                format_sol_err!(e, e.txHash, e.merkleBranchPath, e.merkleBranchHashes),
            ),
            BridgeBtcUnknownError(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.errorCode))
            }
            BridgeExceededLockingCap(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.amount))
            }
            BridgeUnauthorizedCaller(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            ERC1967InvalidImplementation(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.implementation))
            }
            ERC1967NonPayable(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            FailedCall(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            IncorrectInputsNumber(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.expected))
            }
            IncorrectOutputsNumber(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidBtcTxVersion(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidInitialization(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            InvalidLocktime(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidPubKeyLength(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.usrPubKeyLength))
            }
            InvalidSequence(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidVout(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            NoEmptySlot(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(
                e,
                e.packetNumber,
                e.streamId
            )),
            NoFilledSlot(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            NonExistentSlot(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(
                e,
                e.packetNumber,
                e.streamId,
                e.slotId
            )),
            NotEnoughConfirmations(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            NotInitializing(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            OwnableInvalidOwner(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.owner))
            }
            OwnableUnauthorizedAccount(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.account))
            }
            PacketOutOfBound(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.packetNumber))
            }
            PegoutRequestAmountExceedsUint64Limit(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.amount))
            }
            tooManyDenominations(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.maxDenominationsSize))
            }
            UnregisteredPegInRequest(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.btcTxHash))
            }
            UUPSUnauthorizedCallContext(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            UUPSUnsupportedProxiableUUID(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.slot))
            }
        }
    }
}

impl TryFrom<RegisterPegInInput> for BtcTxSPVProof {
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

        Ok(BtcTxSPVProof {
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
        IncorrectOutputScript, SolBitcoinManagerErrors,
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
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputScript(IncorrectOutputScript {
            actual: alloy_primitives::Bytes::from(vec![0x01, 0x2]),
            expected: alloy_primitives::Bytes::from(vec![0x02, 0x3]),
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
