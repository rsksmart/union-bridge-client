use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, FixedBytes, TxHash};
use alloy_provider::Provider;
use anyhow::Result;
#[cfg(test)]
use mockall::automock;
use tracing::{error, info};
use union_contracts::bindings::pegin_manager::PeginManager::{
    self, BtcTransaction, BtcTxSPVProof, PeginManagerErrors, PeginManagerInstance,
};

use crate::contracts::bitcoin_manager::ParseFieldError;
use crate::contracts::common::send_tx_with_gas_bump;
pub(crate) use crate::contracts::interactions::accept_pegin::AcceptPeginInvoke;
pub(crate) use crate::contracts::interactions::get_temporary_pegin_address::GetTemporaryPeginAddressCall;
pub(crate) use crate::contracts::interactions::request_pegin::RequestPeginInvoke;
use crate::rsk_gateway::DomainErrors;
use crate::types::BtcTxSPVProofInput;

#[derive(Clone, Debug)]
pub struct RequestPeginData {
    pub bitcoin_deposit_address: String,
    pub packet_number: u64,
    pub _member_dispute_keys: Vec<FixedBytes<32>>,
    pub _available_slots: u64,
}

#[cfg_attr(test, automock)]
pub trait PeginManagerContractApi {
    async fn call_get_request_pegin_data(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<RequestPeginData>;

    async fn invoke_request_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn invoke_accept_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;
}

#[derive(Clone)]
pub struct PeginManagerContract<P: Provider> {
    contract_instance: PeginManagerInstance<P>,
}

impl<P: Provider> PeginManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to PeginManagerContract @ {contract_address}");
        let contract_instance = PeginManager::new(contract_address, provider);
        PeginManagerContract { contract_instance }
    }
}

impl<P: Provider> PeginManagerContractApi for PeginManagerContract<P> {
    async fn call_get_request_pegin_data(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<RequestPeginData> {
        self.contract_instance
            .getRequestPeginData(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .call()
            .await
            .map(|res| RequestPeginData {
                bitcoin_deposit_address: res.bitcoinDepositAddress,
                packet_number: res.packetNumber,
                _member_dispute_keys: res.memberDisputeKeys,
                _available_slots: res.availableSlots,
            })
    }

    async fn invoke_request_pegin(
        &self,
        input: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
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
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.acceptPegin(input.clone()),
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

    let btc_tx: BtcTransaction = input.btc_tx.try_into().map_err(|e| {
        error!("Failed to parse BTC transaction: {e}");
        e
    })?;

    let merkle_branches_hashes = input
        .merkle_branch_hashes
        .into_iter()
        .map(|hash| hash.parse::<FixedBytes<32>>().map_err(ParseFieldError::ParseHex))
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
    let decoded_err = err.as_decoded_interface_error::<PeginManagerErrors>();
    decoded_err.map(|e| match e {
        PeginManagerErrors::PeginAlreadyAccepted(e) => {
            DomainErrors::PeginAlreadyAccepted(format!("{e:?}"))
        }
        PeginManagerErrors::PeginAlreadyRequested(e) => {
            DomainErrors::PeginAlreadyRequested(format!("{e:?}"))
        }
        PeginManagerErrors::IncorrectOutputsNumber(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PeginManagerErrors::InvalidBtcTxVersion(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        PeginManagerErrors::InvalidLocktime(e) => {
            DomainErrors::InvalidBtcTxSpvProof(format!("{e:?}"))
        }
        // Unhandled
        _ => DomainErrors::UnhandledContractError(format!("{e:?}")),
    })
}
