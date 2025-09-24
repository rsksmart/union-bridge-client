use crate::contracts::{
    bitcoin_manager, committee_registry, member_registry, peg_manager, signature_manager,
    stream_manager,
};
use crate::rsk_gateway::DomainErrors;
use alloy_contract::SolCallBuilder;
use alloy_primitives::{hex::FromHexError, ruint::ParseError};
use alloy_provider::Provider;
use alloy_provider::network::ReceiptResponse;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::SolCall;
use alloy_transport::TransportResult;
use log::{debug, error, warn};
use serde_json::Value;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::timeout;

// Gas bumping constants
const BASE_GAS_HEADROOM_PERCENT: u64 = 120; // 20% base headroom
const PER_ATTEMPT_BUMP_PERCENT: u64 = 110; // 10% per attempt
const OOG_DETECTION_MARGIN_PERCENT: u64 = 5; // 5% margin for OOG detection
const MAX_GAS_LIMIT: u64 = 30_000_000; // Maximum gas limit to prevent resource exhaustion
const DEFAULT_TIMEOUT_SECONDS: u64 = 300; // 5 minutes timeout

#[derive(Debug, Error)]
pub enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

#[derive(Debug, Error)]
pub enum GasBumpError {
    #[error("Gas estimation failed: {0}")]
    GasEstimationFailed(String),
    
    #[error("Transaction timeout after {0} seconds")]
    Timeout(u64),
    
    #[error("Maximum gas limit exceeded: {0} > {1}")]
    MaxGasLimitExceeded(u64, u64),
    
    #[error("Maximum attempts exceeded: {0}")]
    MaxAttemptsExceeded(u8),
    
    #[error("Invalid max_attempts: {0}")]
    InvalidMaxAttempts(u8),
}

// Enhanced send_tx_with_gas_bump with timeout, error handling, and security improvements
pub(super) async fn send_tx_with_gas_bump<P, D, F>(
    provider: &P,
    build_tx: F,
    max_attempts: u8,
) -> alloy_contract::Result<TransactionReceipt>
where
    P: Provider,
    D: SolCall,
    F: Fn() -> SolCallBuilder<P, D>,
{
    // Input validation
    if max_attempts == 0 {
        return Err(alloy_contract::Error::TransportError(
            alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                code: -32602,
                message: GasBumpError::InvalidMaxAttempts(max_attempts).to_string().into(),
                data: None,
            })
        ));
    }

    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(DEFAULT_TIMEOUT_SECONDS);
    
    let mut receipt;
    let mut attempt = 0;
    
    loop {
        // Check timeout
        if start_time.elapsed() > timeout_duration {
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: -32603,
                    message: GasBumpError::Timeout(DEFAULT_TIMEOUT_SECONDS).to_string().into(),
                    data: None,
                })
            ));
        }

        // Enhanced gas estimation with error handling
        // this works also as an eth_call that would check error types, etc., if not do a manual .call()
        let estimated_gas = match timeout(
            Duration::from_secs(30), // 30 second timeout for gas estimation
            build_tx().estimate_gas()
        ).await {
            Ok(Ok(gas)) => gas,
            Ok(Err(e)) => {
                error!("Gas estimation failed: {:?}", e);
                return Err(e);
            },
            Err(_) => {
                error!("Gas estimation timeout");
                return Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: -32603,
                        message: GasBumpError::GasEstimationFailed("Timeout".to_string()).to_string().into(),
                        data: None,
                    })
                ));
            }
        };

        let gas_limit = bumped_gas(estimated_gas, attempt);
        
        // Security check: prevent excessive gas usage
        if gas_limit > MAX_GAS_LIMIT {
            error!("Gas limit {} exceeds maximum allowed {}", gas_limit, MAX_GAS_LIMIT);
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: -32602,
                    message: GasBumpError::MaxGasLimitExceeded(gas_limit, MAX_GAS_LIMIT).to_string().into(),
                    data: None,
                })
            ));
        }

        let gas_price = provider.get_gas_price().await?;
        // let tx_builder = build_tx().gas(gas_limit).gas_price(gas_price).legacy();
        let tx_builder = build_tx().gas(gas_limit).gas_price(gas_price);

        debug!(
            "Sending transaction attempt {} with estimated_gas {} and gas_limit {}",
            attempt + 1, estimated_gas, gas_limit
        );

        // Send transaction with timeout
        let send_result = timeout(
            Duration::from_secs(60), // 60 second timeout for transaction sending
            tx_builder.send()
        ).await;

        let pending_tx = match send_result {
            Ok(Ok(tx)) => tx,
            Ok(Err(e)) => {
                error!("Transaction send failed: {:?}", e);
                return Err(e);
            },
            Err(_) => {
                error!("Transaction send timeout");
                return Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: -32603,
                        message: "Transaction send timeout".to_string().into(),
                        data: None,
                    })
                ));
            }
        };

        // Get receipt with timeout
        let receipt_result = timeout(
            Duration::from_secs(120), // 2 minute timeout for receipt
            pending_tx.get_receipt()
        ).await;

        receipt = match receipt_result {
            Ok(Ok(rec)) => rec,
            Ok(Err(e)) => {
                error!("Failed to get receipt: {:?}", e);
                return Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: -32603,
                        message: format!("Failed to get receipt: {:?}", e).into(),
                        data: None,
                    })
                ));
            },
            Err(_) => {
                error!("Receipt timeout");
                return Err(alloy_contract::Error::TransportError(
                    alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                        code: -32603,
                        message: "Receipt timeout".to_string().into(),
                        data: None,
                    })
                ));
            }
        };

        let should_retry = !receipt.status() 
            && attempt < max_attempts 
            && likely_oog(&receipt, gas_limit);
            
        if should_retry {
            warn!(
                "Transaction failed with OOG, retrying with higher gas. Attempt {}/{}",
                attempt + 1, max_attempts
            );
            attempt += 1;
            continue;
        }

        if receipt.status() {
            debug!("Transaction succeeded after {} attempts: {:?}", attempt + 1, receipt);
        } else {
            // Enhanced error reporting
            let trace_result = timeout(
                Duration::from_secs(30),
                debug_trace_tx(provider, receipt.transaction_hash().to_string())
            ).await.unwrap_or_else(|_| Ok(serde_json::Value::Null));
            
            error!(
                "Transaction failed after {} attempts: {:?} - Trace: {:?}",
                attempt + 1, receipt, trace_result
            );
        }

        break;
    }

    Ok(receipt)
}

// Enhanced gas bumping with constants and better documentation
fn bumped_gas(estimated: u64, attempt: u8) -> u64 {
    // Base headroom: 64/63 to undo the 63/64 rule + 10% for proxy prelude and variance
    // = ~1.117. Use 1.20 to be safe and round up.
    let base = estimated.saturating_mul(BASE_GAS_HEADROOM_PERCENT).saturating_div(100);

    // Per-attempt bump (compounded)
    let mut bumped = base;
    for _ in 0..attempt {
        bumped = bumped.saturating_mul(PER_ATTEMPT_BUMP_PERCENT).saturating_div(100);
    }

    // never go below estimated
    bumped.max(estimated)
}

async fn debug_trace_tx<P: Provider>(provider: &P, tx_hash: String) -> TransportResult<Value> {
    let params = serde_json::json!([
        tx_hash,
        { "tracer": "callTracer" }
    ]);

    provider
        .raw_request("debug_traceTransaction".into(), params)
        .await
}

// Enhanced OOG detection with configurable margin
fn likely_oog(receipt: &TransactionReceipt, gas_limit: u64) -> bool {
    let oog_margin = gas_limit.saturating_mul(OOG_DETECTION_MARGIN_PERCENT).saturating_div(100);
    let oog_candidate =
        !receipt.status() && receipt.gas_used() >= gas_limit.saturating_sub(oog_margin);

    if oog_candidate {
        warn!(
            "Potential OOG detected - Gas used: {}, Gas limit: {}, OOG margin: {} ({}%)",
            receipt.gas_used(),
            gas_limit,
            oog_margin,
            OOG_DETECTION_MARGIN_PERCENT
        );
    };

    oog_candidate
}

impl From<alloy_contract::Error> for DomainErrors {
    fn from(err: alloy_contract::Error) -> Self {
        peg_manager::decode_error(&err)
            .or_else(|| bitcoin_manager::decode_error(&err))
            .or_else(|| stream_manager::decode_error(&err))
            .or_else(|| signature_manager::decode_error(&err))
            .or_else(|| committee_registry::decode_error(&err))
            .or_else(|| member_registry::decode_error(&err))
            .unwrap_or_else(|| DomainErrors::NoRevertError(format!("{:?}", err)))
    }
}

#[cfg(test)]
mod common_tests;

#[cfg(test)]
pub(crate) mod tests {
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::ErrorPayload;
    use alloy_json_rpc::RpcError::ErrorResp;
    use alloy_sol_types::{SolInterface, SolValue};

    pub(crate) const CONTRACT_ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

    #[allow(dead_code)]
    pub(crate) const NO_REVERT_ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"<to_replace_message>:","data":"<to_replace_data>"}"#;

    pub(crate) fn generate_contract_revert_error<T: SolInterface>(
        input: T,
    ) -> alloy_contract::Error {
        let error = CONTRACT_ERROR_TEMPLATE.replace(
            "<to_replace>",
            &format!("0x{}", hex::encode(input.abi_encode())),
        );
        let payload = serde_json::from_str::<ErrorPayload>(&error).unwrap();
        TransportError(ErrorResp(payload))
    }

    #[allow(dead_code)]
    pub(crate) fn generate_no_revert_error(msg: &str, data: &str) -> alloy_contract::Error {
        let error = CONTRACT_ERROR_TEMPLATE
            .replace(
                "<to_replace_message>",
                &format!("0x{}", hex::encode(msg.abi_encode())),
            )
            .replace(
                "<to_replace_data>",
                &format!("0x{}", hex::encode(data.abi_encode())),
            );
        let payload = serde_json::from_str::<ErrorPayload>(&error).unwrap();
        TransportError(ErrorResp(payload))
    }
}
