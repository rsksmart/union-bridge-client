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
use std::rc::Rc;

use common::msg_broker::bitvmx_types::BtcTxSPVProof;
use common::runtime_sync::RuntimeSync;
use common::types::{Hash256, TxIdParser};
use log::{debug, info, warn};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::GetBtcTransactionConfirmationsInput;

pub const MIN_TX_CONFIRMATIONS: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum VerificationStatus {
    Verified,
    InsufficientConfirmations { required: u32, actual: u32 },
}

pub enum NativeBridgeVerifier<CG: RskContractsGatewayApi> {
    Real { contracts: Rc<CG>, rt_sync: RuntimeSync },
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

const NATIVE_BRIDGE_ERROR_MARKER: &str = "Native Bridge returned error code:";

fn parse_native_bridge_error_code(err: &DomainErrors) -> Option<i32> {
    let DomainErrors::UnhandledContractError(message) = err else {
        return None;
    };

    let (_, rest) = message.split_once(NATIVE_BRIDGE_ERROR_MARKER)?;
    let code_str = rest.split_whitespace().next()?;
    code_str.parse().ok()
}

fn native_bridge_error_reason(code: i32) -> Option<&'static str> {
    match code {
        -1 => Some("block hash not found in bridge view"),
        -2 => Some("block is not in best chain"),
        -3 => Some("internal inconsistency prevented block lookup"),
        -4 => Some("block too old to be processed"),
        -5 => Some("merkle branch does not prove inclusion"),
        _ => None,
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
                return Err(DomainErrors::InvalidBtcTxSpvProof(format!("Invalid block hash: {e}")));
            }
        };

    let tx_id = spv_proof.tx.compute_txid();
    let tx_hash_fb = TxIdParser::txid_to_fb_32(tx_id);
    let tx_hash: common::types::TxHash =
        Hash256::from(primitive_types::H256::from_slice(tx_hash_fb.as_slice()));

    let merkle_branch_hashes: Vec<String> =
        spv_proof.merkle_branch_hashes.iter().map(hex::encode).collect();

    let input = GetBtcTransactionConfirmationsInput {
        tx_hash,
        block_hash,
        merkle_branch_path: spv_proof.merkle_branch_path.clone(),
        merkle_branch_hashes,
    };

    // query native bridge
    match invoke_contract(rt_sync, "getBtcTransactionConfirmations", || async {
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
            // NOTE: Keep this stable-friendly; Docker builds use stable rustc (no let-chains).
            // Discuss whether we want to move Docker to nightly if we really need let-chains.
            if let Some((code, reason)) = parse_native_bridge_error_code(&e)
                .and_then(|code| native_bridge_error_reason(code).map(|reason| (code, reason)))
            {
                warn!("Native Bridge returned error code {code}: {reason}. Will retry later");
                return Ok(VerificationStatus::InsufficientConfirmations {
                    required: required_confirmations,
                    actual: 0,
                });
            }

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
    debug!("Verifying Native Bridge confirmations before invoking contract");

    let res = native_bridge_verifier.verify_confirmations(spv_proof, MIN_TX_CONFIRMATIONS)?;

    match res {
        VerificationStatus::Verified => {
            debug!("Invoke contract. method={method_name}");
            invoke_contract(rt_sync, method_name, invoke)
        }
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
