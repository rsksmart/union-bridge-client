use crate::contracts::{
    common::send_tx_with_gas_bump,
    peg_manager::SolPegManager::{
        BtcTransaction, BtcTxSPVProof, SolPegManagerErrors, SolPegManagerInstance,
        getTemporaryPegInAddressReturn,
    },
};

use alloy_json_rpc::ErrorPayload;
use alloy_primitives::{Address, FixedBytes, U256, hex::FromHex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::SolInterface;
use anyhow::Result;
use log::error;

use crate::contracts::bitcoin_manager::ParseFieldError;

use crate::format_sol_err;
use crate::rsk_gateway::PegManagerErrors;
use crate::types::RegisterPegInInput;

// re-export for convenience
use crate::contracts::bitcoin_manager::SolBitcoinManager::SolBitcoinManagerErrors;
pub(crate) use crate::contracts::interactions::accept_peg_in_request;
pub(crate) use crate::contracts::interactions::get_temporary_peg_in_address;
pub(crate) use crate::contracts::interactions::register_peg_in_request;
pub(crate) use crate::contracts::interactions::register_peg_out_request;

use SolPegManagerErrors::*;

#[cfg(test)]
use mockall::automock;

include!(concat!(env!("OUT_DIR"), "/abi.rs"));

#[cfg_attr(test, automock)]
pub trait PegManagerContractApi {
    async fn get_temporary_peg_in_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;

    async fn register_peg_in_request_send(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn accept_peg_in_request_send(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn register_peg_out_request_send(
        &self,
        msg_value: u64,
        usr_pub_key: FixedBytes<33>,
        batch_flag: bool,
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
            None,
        )
        .await
    }

    async fn accept_peg_in_request_send(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            || self.contract_instance.acceptPegInRequest(input.clone()),
            gas_bumps,
            None,
        )
        .await
    }

    async fn register_peg_out_request_send(
        &self,
        msg_value: u64,
        usr_pub_key: FixedBytes<33>,
        batch_flag: bool,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            || {
                self.contract_instance
                    .requestPegOut(usr_pub_key.into(), batch_flag)
            },
            gas_bumps,
            Some(msg_value),
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
            // Mapped explicitly to specific PegManagerErrors variants
            AlreadyRegisteredPegIn(e) => {
                PegManagerErrors::AlreadyRegisteredPegIn(format_sol_err!(e, e.btcTxHash))
            }
            AlreadyRegisteredPegInRequest(e) => {
                PegManagerErrors::AlreadyRegisteredPegInRequest(format_sol_err!(e, e.btcTxHash))
            }
            StreamNotFoundByDenomination(e) => {
                PegManagerErrors::StreamNotFoundByDenomination(format_sol_err!(e, e.denomination))
            }
            AlreadyRegisteredAcceptPegIn(e) => {
                PegManagerErrors::AlreadyRegisteredAcceptPegIn(format_sol_err!(e, e.btcTxHash))
            }
            IncorrectInputsNumber(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            IncorrectOutputsNumber(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidBtcTxVersion(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidLocktime(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidSequence(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            InvalidVout(e) => {
                PegManagerErrors::InvalidBtcTxSpvProof(format_sol_err!(e, e.expected, e.actual))
            }
            NotEnoughConfirmations(e) => {
                PegManagerErrors::NotEnoughConfirmations(format_sol_err!(e, e.expected, e.actual))
            }
            PacketOutOfBound(e) => {
                PegManagerErrors::PacketOutOfBound(format_sol_err!(e, e.packetNumber))
            }
            UnregisteredPegInRequest(e) => {
                PegManagerErrors::UnregisteredRequest(format_sol_err!(e, e.btcTxHash))
            }

            // Defaulted to UnhandledContractError (still with specific data formatting)
            AddressEmptyCode(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.target))
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
            InvalidInitialization(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            InvalidPubKeyLength(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.usrPubKeyLength))
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
            NotInitializing(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            OwnableInvalidOwner(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.owner))
            }
            OwnableUnauthorizedAccount(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.account))
            }
            PegoutRequestAmountExceedsUint64Limit(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.amount))
            }
            tooManyDenominations(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.maxDenominationsSize))
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
        AlreadyRegisteredAcceptPegIn, AlreadyRegisteredPegIn, AlreadyRegisteredPegInRequest,
        BridgeBtcBlockNotInBestChain, IncorrectInputsNumber, IncorrectOutputsNumber,
        InvalidBtcTxVersion, InvalidLocktime, InvalidSequence, InvalidVout, NotInitializing,
        PacketOutOfBound, SolPegManagerErrors, StreamNotFoundByDenomination,
        UnregisteredPegInRequest,
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

    #[test]
    fn test_already_registered_accept_peg_in() {
        let expected_err =
            SolPegManagerErrors::AlreadyRegisteredAcceptPegIn(AlreadyRegisteredAcceptPegIn {
                btcTxHash: "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
                    .parse()
                    .expect("Failed to parse tx hash"),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::AlreadyRegisteredAcceptPegIn(_));
    }

    #[test]
    fn test_invalid_btc_tx_version() {
        let expected_err = SolPegManagerErrors::InvalidBtcTxVersion(InvalidBtcTxVersion {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_bridge_btc_block_not_in_best_chain() {
        let expected_err =
            SolPegManagerErrors::BridgeBtcBlockNotInBestChain(BridgeBtcBlockNotInBestChain {
                blockHash: "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c"
                    .parse()
                    .expect("Failed to parse block hash"),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::UnhandledContractError(_));
    }

    #[test]
    fn test_unregistered_peg_in_request() {
        let expected_err =
            SolPegManagerErrors::UnregisteredPegInRequest(UnregisteredPegInRequest {
                btcTxHash: "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
                    .parse()
                    .unwrap(),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::UnregisteredRequest(_));
    }

    #[test]
    fn test_incorrect_inputs_number() {
        let expected_err = SolPegManagerErrors::IncorrectInputsNumber(IncorrectInputsNumber {
            expected: alloy_primitives::U256::from(2),
            actual: alloy_primitives::U256::from(3),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_incorrect_outputs_number() {
        let expected_err = SolPegManagerErrors::IncorrectOutputsNumber(IncorrectOutputsNumber {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_locktime() {
        let expected_err = SolPegManagerErrors::InvalidLocktime(InvalidLocktime {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_sequence() {
        let expected_err = SolPegManagerErrors::InvalidSequence(InvalidSequence {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_vout() {
        let expected_err = SolPegManagerErrors::InvalidVout(InvalidVout {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_packet_out_of_bound() {
        let expected_err = SolPegManagerErrors::PacketOutOfBound(PacketOutOfBound {
            packetNumber: alloy_primitives::U256::from(42),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::PacketOutOfBound(_));
    }

    #[test]
    fn test_already_registered_peg_in_request() {
        let expected_err =
            SolPegManagerErrors::AlreadyRegisteredPegInRequest(AlreadyRegisteredPegInRequest {
                btcTxHash: "0x987654321abcdef987654321abcdef987654321abcdef987654321abcdef9876"
                    .parse()
                    .unwrap(),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::AlreadyRegisteredPegInRequest(_));
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

        matches!(result, PegManagerErrors::InvalidBtcTxSpvProof(_));
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
