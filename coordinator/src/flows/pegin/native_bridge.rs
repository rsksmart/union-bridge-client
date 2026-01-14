fn invoke_contract<Fut, F, T>(
    rt_sync: &RuntimeSync,
    method_name: &str,
    invoke: F,
) -> Result<T, DomainErrors>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, DomainErrors>>,
{
    debug!("Submitting contract transaction: method={method_name}");

    match rt_sync.run(invoke()) {
        Ok(value) => {
            debug!("Contract method executed: method={method_name}");
            Ok(value)
        }
        Err(domain_err) => Err(domain_err),
    }
}
use common::{
    msg_broker::bitvmx_types::BtcTxSPVProof,
    runtime_sync::RuntimeSync,
    types::{Hash256, TxIdParser},
};
use log::{debug, info, warn};
use std::rc::Rc;
use transaction_dispatcher::{
    rsk_gateway::{DomainErrors, RskContractsGatewayApi},
    types::GetBtcTransactionConfirmationsInput,
};

pub const MIN_TX_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node

#[derive(Debug, Clone, PartialEq)]
pub enum VerificationStatus {
    Verified,
    InsufficientConfirmations { required: u32, actual: u32 },
}

pub enum NativeBridgeVerifier<CG: RskContractsGatewayApi> {
    Real {
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
    },
    Dummy, // used in local/test environments
}

impl<CG: RskContractsGatewayApi> Clone for NativeBridgeVerifier<CG> {
    fn clone(&self) -> Self {
        match self {
            NativeBridgeVerifier::Real { contracts, rt_sync } => NativeBridgeVerifier::Real {
                contracts: Rc::clone(contracts),
                rt_sync: rt_sync.clone(),
            },
            NativeBridgeVerifier::Dummy => NativeBridgeVerifier::Dummy,
        }
    }
}

impl<CG: RskContractsGatewayApi> NativeBridgeVerifier<CG> {
    pub fn verify_confirmations(
        &self,
        spv_proof: &BtcTxSPVProof,
        required_confirmations: u32,
    ) -> Result<VerificationStatus, DomainErrors> {
        match self {
            NativeBridgeVerifier::Real { contracts, rt_sync } => {
                verify_btc_confirmations(spv_proof, required_confirmations, contracts, rt_sync)
            }
            NativeBridgeVerifier::Dummy => {
                debug!(
                    "Using dummy verifier: skipping Native Bridge verification (local environment)"
                );
                Ok(VerificationStatus::Verified)
            }
        }
    }
}

fn verify_btc_confirmations<CG>(
    spv_proof: &BtcTxSPVProof,
    required_confirmations: u32,
    contracts: &Rc<CG>,
    rt_sync: &RuntimeSync,
) -> Result<VerificationStatus, DomainErrors>
where
    CG: RskContractsGatewayApi,
{
    let block_hash: common::types::BlockHash =
        match spv_proof.block_hash.parse::<primitive_types::H256>() {
            Ok(h) => h.into(),
            Err(e) => {
                warn!("Failed to parse block hash: {e}");
                return Err(DomainErrors::InvalidBtcTxSpvProof(format!(
                    "Invalid block hash: {e}"
                )));
            }
        };

    let tx_id = spv_proof.tx.compute_txid();
    let tx_hash_fb = TxIdParser::txid_to_fb_32(tx_id);
    let tx_hash: common::types::TxHash =
        Hash256::from(primitive_types::H256::from_slice(tx_hash_fb.as_slice()));

    let merkle_branch_hashes: Vec<String> = spv_proof
        .merkle_branch_hashes
        .iter()
        .map(hex::encode)
        .collect();

    let input = GetBtcTransactionConfirmationsInput {
        tx_hash,
        block_hash,
        merkle_branch_path: spv_proof.merkle_branch_path.clone(),
        merkle_branch_hashes,
    };

    // query native bridge
    match invoke_contract(rt_sync, "getBtcConfirmations", || async {
        contracts.get_btc_confirmations(input).await
    }) {
        Ok(output) => {
            let confirmations = output.confirmations;
            if confirmations >= required_confirmations {
                debug!(
                    "Native Bridge verification passed: {confirmations}/{required_confirmations} confirmations"
                );
                Ok(VerificationStatus::Verified)
            } else {
                info!(
                    "Native Bridge has insufficient confirmations: {confirmations}/{required_confirmations}, will retry later"
                );
                // Insufficient confirmations is NOT an error, it's an expected state
                Ok(VerificationStatus::InsufficientConfirmations {
                    required: required_confirmations,
                    actual: confirmations,
                })
            }
        }
        Err(e) => {
            warn!("Native Bridge query failed: {e}");
            Err(e)
        }
    }
}

/// Verifies that the native bridge has enough confirmations for the given SPV proof
/// and then invokes a contract method
pub fn invoke_contract_safe<Fut, F, T, CG>(
    rt_sync: &RuntimeSync,
    method_name: &str,
    spv_proof: &BtcTxSPVProof,
    native_bridge_verifier: &NativeBridgeVerifier<CG>,
    invoke: F,
) -> Result<T, DomainErrors>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, DomainErrors>>,
    CG: RskContractsGatewayApi,
{
    debug!("Verifying Native Bridge confirmations before invoking: method={method_name}");

    match native_bridge_verifier.verify_confirmations(spv_proof, MIN_TX_CONFIRMATIONS)? {
        VerificationStatus::Verified => invoke_contract(rt_sync, method_name, invoke),
        VerificationStatus::InsufficientConfirmations { required, actual } => {
            debug!(
                "Insufficient Native Bridge confirmations for {method_name}: {actual}/{required} - needs retry"
            );
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(format!(
                "{actual}/{required} confirmations"
            )))
        }
    }
}
