use crate::contracts::common::send_tx_with_gas_bump;
use alloy_primitives::{Address, FixedBytes, U256, hex::FromHex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use anyhow::Result;
use log::{error, info};
use union_contracts::bindings::peg_manager::PegManager::{
    self, BtcTransaction, BtcTxSPVProof, PegManagerErrors, PegManagerInstance,
};

use crate::contracts::bitcoin_manager::ParseFieldError;

use crate::types::BtcTxSPVProofInput;

// re-export for convenience
pub(crate) use crate::contracts::interactions::accept_pegin;
pub(crate) use crate::contracts::interactions::get_temporary_pegin_address;
pub(crate) use crate::contracts::interactions::notify_check_fork_complete;
pub(crate) use crate::contracts::interactions::register_pegout;
pub(crate) use crate::contracts::interactions::request_pegin;
pub(crate) use crate::contracts::interactions::request_pegout;

use crate::rsk_gateway::DomainErrors;
use actors_mocking::fake_contracts::FakePegManager;
use actors_mocking::fake_contracts::FakePegManager::FakePegManagerInstance;
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait PegManagerContractApi {
    async fn call_get_temporary_pegin_address(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<String>;

    async fn invoke_request_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn invoke_accept_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn invoke_request_pegout(
        &self,
        msg_value: u64,
        usr_pub_key: FixedBytes<33>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn invoke_register_pegout(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn notify_check_fork_completion(
        &self,
        pegout_id: &str,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;
}

// needed so we can create a PegManagerContractApi trait for tests mocking
#[derive(Clone)]
pub struct PegManagerContract<P: Provider> {
    contract_instance: PegManagerInstance<P>,
}

impl<P: Provider> PegManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to PegManagerContract @ {}", contract_address);
        let contract_instance = PegManager::new(contract_address, provider);
        PegManagerContract { contract_instance }
    }
}

impl<P: Provider> PegManagerContractApi for PegManagerContract<P> {
    async fn call_get_temporary_pegin_address(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<String> {
        self.contract_instance
            .getTemporaryPeginAddress(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .call()
            .await
    }

    async fn invoke_request_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.requestPegin(input.clone()),
            gas_bumps,
        )
        .await
    }

    async fn invoke_accept_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.acceptPegin(input.clone()),
            gas_bumps,
        )
        .await
    }

    async fn invoke_request_pegout(
        &self,
        msg_value: u64,
        usr_pub_key: FixedBytes<33>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || {
                self.contract_instance
                    .tryPegout(usr_pub_key.into())
                    .value(U256::from(msg_value))
            },
            gas_bumps,
        )
        .await
    }

    async fn invoke_register_pegout(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.registerOperatorTake(input.clone()),
            gas_bumps,
        )
        .await
    }

    async fn notify_check_fork_completion(
        &self,
        _pegout_id: &str,
        _gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        todo!("NotifyCheckForkComplete is not implemented yet for real PegManager");
    }
}

// needed so we can create a PegManagerContractApi trait for tests mocking
#[derive(Clone)]
pub struct FakePegManagerContract<P: Provider> {
    contract_instance: FakePegManagerInstance<P>,
}

impl<P: Provider> FakePegManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!(
            "Connecting to FakePegManagerContract @ {}",
            contract_address
        );
        let contract_instance = FakePegManager::new(contract_address, provider);
        FakePegManagerContract { contract_instance }
    }
}

impl<P: Provider> PegManagerContractApi for FakePegManagerContract<P> {
    async fn call_get_temporary_pegin_address(
        &self,
        _rootstock_deposit_address: Address,
        _value: u64,
        _btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<String> {
        todo!("Not yet implemented for FakePegManagerContract");
    }

    async fn invoke_request_pegin(
        &self,
        _input: BtcTxSPVProof,
        _gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        todo!("Not yet implemented for FakePegManagerContract");
    }

    async fn invoke_accept_pegin(
        &self,
        _input: BtcTxSPVProof,
        _gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        todo!("Not yet implemented for FakePegManagerContract");
    }

    async fn invoke_request_pegout(
        &self,
        _msg_value: u64,
        _usr_pub_key: FixedBytes<33>,
        _gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        todo!("Not yet implemented for FakePegManagerContract");
    }
    async fn invoke_register_pegout(
        &self,
        _input: BtcTxSPVProof,
        _gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        todo!("Not yet implemented for FakePegManagerContract");
    }

    async fn notify_check_fork_completion(
        &self,
        pegout_id: &str,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || {
                self.contract_instance
                    .checkForkComplete(pegout_id.to_string())
            },
            gas_bumps,
        )
        .await
    }
}

impl TryFrom<BtcTxSPVProofInput> for BtcTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: BtcTxSPVProofInput) -> Result<Self, Self::Error> {
        value.into_btc_tx_spv_proof()
    }
}

impl BtcTxSPVProofInput {
    fn into_btc_tx_spv_proof(self) -> Result<BtcTxSPVProof, ParseFieldError> {
        let block_hash =
            FixedBytes::<32>::from_hex(&self.block_hash).map_err(ParseFieldError::ParseHex)?;

        let btc_tx: BtcTransaction = self.btc_tx.try_into().map_err(|e| {
            error!("Failed to parse BTC transaction: {}", e);
            e
        })?;

        let merkle_branches_hashes = self
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
            U256::from_str_radix(&self.merkle_branch_path.trim_start_matches("0x"), 16).map_err(
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

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<PegManagerErrors>();
    decoded_err.map(|e| match e {
        PegManagerErrors::PeginAlreadyAccepted(e) => {
            DomainErrors::PeginAlreadyAccepted(format!("{:?}", e))
        }
        PegManagerErrors::PeginAlreadyRequested(e) => {
            DomainErrors::PeginAlreadyRequested(format!("{:?}", e))
        }
        PegManagerErrors::IncorrectInputsNumber(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
        }
        PegManagerErrors::IncorrectOutputsNumber(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
        }
        PegManagerErrors::InvalidBtcTxVersion(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
        }
        PegManagerErrors::InvalidLocktime(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{:?}", e))
        }
        PegManagerErrors::InvalidCompressedPubKey(e) => {
            DomainErrors::InvalidCompressedPubKey(format!("{:?}", e))
        }
        PegManagerErrors::PegoutRequestAmountExceedsUint64Limit(e) => {
            DomainErrors::PegoutRequestAmountExceedsUint64Limit(format!("{:?}", e))
        }
        // Native Bridge Errors
        PegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
            // we consider this reversible, so we map it to MissingConfirmationsOnNativeBridge
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
        }
        PegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
        }
        PegManagerErrors::NotEnoughConfirmations(e) => {
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{:?}", e))
        }
        // Unhandled
        _ => DomainErrors::UnhandledContractError(format!("{:?}", e)),
    })
}

#[cfg(test)]
mod tests {
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;
    use union_contracts::bindings::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, IncorrectOutputScript,
    };
    use union_contracts::bindings::peg_manager::PegManager::{
        BridgeBtcBlockNotInBestChain, IncorrectInputsNumber, IncorrectOutputsNumber,
        InvalidBtcTxVersion, InvalidLocktime, NotInitializing, PegManagerErrors,
        PeginAlreadyRequested,
    };

    #[test]
    fn test_already_registered_accept_pegin() {
        let expected_err = PegManagerErrors::PeginAlreadyRequested(PeginAlreadyRequested {
            btcTxHash: "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
                .parse()
                .expect("Failed to parse tx hash"),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::PeginAlreadyRequested(_));
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
    fn test_already_pegin_requested() {
        let expected_err = PegManagerErrors::PeginAlreadyRequested(PeginAlreadyRequested {
            btcTxHash: "0x987654321abcdef987654321abcdef987654321abcdef987654321abcdef9876"
                .parse()
                .unwrap(),
        });

        let result = generate_contract_revert_error(expected_err);
        matches!(result.into(), DomainErrors::PeginAlreadyRequested(_));
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
