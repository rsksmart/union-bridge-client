use crate::contracts::bitcoin_manager::ParseFieldError;
use crate::contracts::common::send_tx_with_gas_bump;
use crate::rsk_gateway::DomainErrors;
use crate::types::BtcTxSPVProofInput;
use alloy_primitives::{Address, Bytes, FixedBytes, U256, hex::FromHex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use log::{error, info};
use union_contracts::bindings::pegout_manager::PegoutManager::{
    self, BtcTransaction, BtcTxIn, BtcTxOut, BtcTxSPVProof, PegoutManagerErrors,
    PegoutManagerInstance,
};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait PegoutManagerContractApi {
    async fn invoke_try_pegout(
        &self,
        msg_value: u64,
        usr_pub_key: FixedBytes<33>,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;

    async fn invoke_register_user_take(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt>;
}

#[derive(Clone)]
pub struct PegoutManagerContract<P: Provider> {
    contract_instance: PegoutManagerInstance<P>,
}

impl<P: Provider> PegoutManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to PegoutManagerContract @ {contract_address}");
        let contract_instance = PegoutManager::new(contract_address, provider);
        PegoutManagerContract { contract_instance }
    }
}

impl<P: Provider> PegoutManagerContractApi for PegoutManagerContract<P> {
    async fn invoke_try_pegout(
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

    async fn invoke_register_user_take(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TransactionReceipt> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.registerUserTake(input.clone()),
            gas_bumps,
        )
        .await
    }
}

impl TryFrom<BtcTxSPVProofInput> for BtcTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: BtcTxSPVProofInput) -> Result<Self, Self::Error> {
        build_btc_tx_spv_proof(value)
    }
}

fn build_btc_tx_spv_proof(input: BtcTxSPVProofInput) -> Result<BtcTxSPVProof, ParseFieldError> {
    let block_hash =
        FixedBytes::<32>::from_hex(&input.block_hash).map_err(ParseFieldError::ParseHex)?;

    let inputs = input
        .btc_tx
        .inputs
        .into_iter()
        .map(|i| {
            let txid = i.tx_id.parse().map_err(ParseFieldError::ParseHex)?;
            let script_sig = Bytes::from_hex(i.script_sig).map_err(ParseFieldError::ParseHex)?;
            Ok(BtcTxIn {
                txId: txid,
                vout: i.v_out,
                sequence: i.sequence,
                scriptSig: script_sig,
            })
        })
        .collect::<Result<Vec<BtcTxIn>, ParseFieldError>>()?;

    let outputs = input
        .btc_tx
        .outputs
        .into_iter()
        .map(|o| {
            let script_pub_key =
                Bytes::from_hex(o.script_pub_key).map_err(ParseFieldError::ParseHex)?;
            Ok(BtcTxOut {
                amount: o.amount,
                scriptPubKey: script_pub_key,
            })
        })
        .collect::<Result<Vec<BtcTxOut>, ParseFieldError>>()?;

    let btc_tx = BtcTransaction {
        version: input.btc_tx.version,
        inputs,
        outputs,
        locktime: input.btc_tx.lock_time,
    };

    let merkle_branches_hashes = input
        .merkle_branch_hashes
        .into_iter()
        .map(|hash| {
            hash.parse::<FixedBytes<32>>()
                .map_err(ParseFieldError::ParseHex)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            error!("Failed to convert merkle_branch_hashes: {e:?}");
            e
        })?;

    Ok(BtcTxSPVProof {
        blockHash: block_hash,
        btcTx: btc_tx,
        merkleBranchPath: input.merkle_branch_path.parse()?,
        merkleBranchHashes: merkle_branches_hashes,
    })
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<PegoutManagerErrors>();
    decoded_err.map(|e| match e {
        PegoutManagerErrors::InvalidCompressedPubKey(e) => {
            DomainErrors::InvalidCompressedPubKey(format!("{e:?}"))
        }
        PegoutManagerErrors::PegoutRequestAmountExceedsUint64Limit(e) => {
            DomainErrors::PegoutRequestAmountExceedsUint64Limit(format!("{e:?}"))
        }
        PegoutManagerErrors::IncorrectInputsNumber(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PegoutManagerErrors::IncorrectOutputsNumber(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PegoutManagerErrors::InvalidBtcTxVersion(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PegoutManagerErrors::InvalidLocktime(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PegoutManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{e:?}"))
        }
        PegoutManagerErrors::BridgeBtcInexistantBlockHash(e) => {
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{e:?}"))
        }
        PegoutManagerErrors::NotEnoughConfirmations(e) => {
            DomainErrors::MissingConfirmationsOnNativeBridge(format!("{e:?}"))
        }
        PegoutManagerErrors::InvalidSlotState(e) => DomainErrors::InvalidSlotState {
            expected: e.expected.into(),
            actual: e.actual.into(),
        },
        // Unhandled
        _ => DomainErrors::UnhandledContractError(format!("{e:?}")),
    })
}
