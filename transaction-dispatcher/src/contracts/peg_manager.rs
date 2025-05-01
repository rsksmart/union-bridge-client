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
use anyhow::Result;
use log::error;
use union_contracts::bindings::pegmanager::PegManager;
use union_contracts::bindings::pegmanager::PegManager::{
    BtcTransaction, BtcTxSPVProof, PegManagerInstance, getTemporaryPegInAddressReturn,
};

use crate::contracts::bitcoin_manager::ParseFieldError;

use crate::types::RegisterPegInInput;

// re-export for convenience
pub(crate) use crate::contracts::interactions::accept_peg_in_request;
pub(crate) use crate::contracts::interactions::get_temporary_peg_in_address;
pub(crate) use crate::contracts::interactions::register_peg_in_request;
pub(crate) use crate::contracts::interactions::register_peg_out_request;

#[cfg(test)]
use mockall::automock;

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
    contract_instance: PegManagerInstance<(), P>,
}

impl<P: Provider> PegManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        let contract_instance = PegManager::new(contract_address, provider);
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

    async fn accept_peg_in_request_send(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            || self.contract_instance.acceptPegInRequest(input.clone()),
            gas_bumps,
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
                    .value(U256::from(msg_value))
            },
            gas_bumps,
        )
        .await
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
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;
    use union_contracts::bindings::bitcoinmanager::BitcoinManager::{
        BitcoinManagerErrors, IncorrectOutputScript,
    };
    use union_contracts::bindings::pegmanager::PegManager::{
        AlreadyRegisteredAcceptPegIn, AlreadyRegisteredPegIn, AlreadyRegisteredPegInRequest,
        BridgeBtcBlockNotInBestChain, IncorrectInputsNumber, IncorrectOutputsNumber,
        InvalidBtcTxVersion, InvalidLocktime, InvalidSequence, InvalidVout, NotInitializing,
        PacketOutOfBound, PegManagerErrors, StreamNotFoundByDenomination, UnregisteredPegInRequest,
    };

    #[test]
    fn test_already_registered_peg_in() {
        let expected_err = PegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
            btcTxHash: "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e"
                .parse()
                .expect("Failed to parse tx hash"),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::AlreadyRegisteredPegIn(_));
    }

    #[test]
    fn test_stream_not_found_by_denomination() {
        let expected_err =
            PegManagerErrors::StreamNotFoundByDenomination(StreamNotFoundByDenomination {
                denomination: alloy_primitives::Uint::from(125),
            });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::StreamNotFoundByDenomination(_));
    }

    #[test]
    fn test_already_registered_accept_peg_in() {
        let expected_err =
            PegManagerErrors::AlreadyRegisteredAcceptPegIn(AlreadyRegisteredAcceptPegIn {
                btcTxHash: "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
                    .parse()
                    .expect("Failed to parse tx hash"),
            });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::AlreadyRegisteredAcceptPegIn(_));
    }

    #[test]
    fn test_invalid_btc_tx_version() {
        let expected_err = PegManagerErrors::InvalidBtcTxVersion(InvalidBtcTxVersion {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_bridge_btc_block_not_in_best_chain() {
        let expected_err =
            PegManagerErrors::BridgeBtcBlockNotInBestChain(BridgeBtcBlockNotInBestChain {
                blockHash: "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c"
                    .parse()
                    .expect("Failed to parse block hash"),
            });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::UnhandledContractError(_));
    }

    #[test]
    fn test_unregistered_peg_in_request() {
        let expected_err = PegManagerErrors::UnregisteredPegInRequest(UnregisteredPegInRequest {
            btcTxHash: "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
                .parse()
                .unwrap(),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::UnregisteredRequest(_));
    }

    #[test]
    fn test_incorrect_inputs_number() {
        let expected_err = PegManagerErrors::IncorrectInputsNumber(IncorrectInputsNumber {
            expected: alloy_primitives::U256::from(2),
            actual: alloy_primitives::U256::from(3),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_incorrect_outputs_number() {
        let expected_err = PegManagerErrors::IncorrectOutputsNumber(IncorrectOutputsNumber {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_locktime() {
        let expected_err = PegManagerErrors::InvalidLocktime(InvalidLocktime {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_sequence() {
        let expected_err = PegManagerErrors::InvalidSequence(InvalidSequence {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_vout() {
        let expected_err = PegManagerErrors::InvalidVout(InvalidVout {
            expected: alloy_primitives::U256::from(1),
            actual: alloy_primitives::U256::from(2),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_packet_out_of_bound() {
        let expected_err = PegManagerErrors::PacketOutOfBound(PacketOutOfBound {
            packetNumber: alloy_primitives::U256::from(42),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::PacketOutOfBound(_));
    }

    #[test]
    fn test_already_registered_peg_in_request() {
        let expected_err =
            PegManagerErrors::AlreadyRegisteredPegInRequest(AlreadyRegisteredPegInRequest {
                btcTxHash: "0x987654321abcdef987654321abcdef987654321abcdef987654321abcdef9876"
                    .parse()
                    .unwrap(),
            });

        let result = generate_contract_revert_error(expected_err);
        matches!(
            result.into(),
            DomainErrors::AlreadyRegisteredPegInRequest(_)
        );
    }

    // check one of the errors to ensure the code keeps covering also BitcoinManagerErrors
    // but the tests for BitcoinManagerErrors are in the bitcoin_manager.rs
    #[test]
    fn test_bitcoin_manager_error() {
        let expected_err = BitcoinManagerErrors::IncorrectOutputScript(IncorrectOutputScript {
            actual: alloy_primitives::Bytes::from(vec![0x01, 0x2]),
            expected: alloy_primitives::Bytes::from(vec![0x02, 0x3]),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    // check one of the errors to ensure the mapping to InternalError keeps working
    // there are more errors that map to InternalError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = PegManagerErrors::NotInitializing(NotInitializing {});

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::UnhandledContractError(_));
    }
}
