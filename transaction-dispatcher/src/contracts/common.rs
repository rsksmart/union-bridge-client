use crate::contracts::{
    bitcoin_manager, committee_registry, member_registry, peg_manager, signature_manager,
    stream_manager,
};
use crate::rsk_gateway::DomainErrors;
use alloy_contract::{CallBuilder, SolCallBuilder};
use alloy_primitives::{hex::FromHexError, ruint::ParseError};
use alloy_provider::Provider;
use alloy_provider::network::ReceiptResponse;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::SolCall;
use alloy_transport::TransportResult;
use log::{debug, error, warn};
use serde_json::Value;
use std::marker::PhantomData;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::timeout;

// Gas bumping constants
const BASE_GAS_HEADROOM_PERCENT: u64 = 120; // 20% base headroom
const PER_ATTEMPT_BUMP_PERCENT: u64 = 110; // 10% per attempt
const OOG_DETECTION_MARGIN_PERCENT: u64 = 5; // 5% margin for OOG detection
const MAX_GAS_LIMIT: u64 = 5_000_000; // 6.8 million block limit in Rootstock
// Ethereum JSON-RPC error codes
const ETH_RPC_INTERNAL_ERROR: i64 = -32603;
const ETH_RPC_INVALID_PARAMS: i64 = -32602;
const ETH_RPC_TIMEOUT: i64 = -32002;

#[derive(Debug, Error)]
pub enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

#[derive(Debug, Error)]
pub enum GasBumpError {
    #[error("Transaction timeout after {0} seconds")]
    Timeout(u64),

    #[error("Maximum gas limit exceeded: {0} > {1}")]
    MaxGasLimitExceeded(u64, u64),
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
    // Input validation - max_attempts represents retries, so 0 is valid (1 attempt, 0 retries)
    // No validation needed as max_attempts = 0 means "no retries, just one attempt"

    let start_time = Instant::now();

    let mut receipt = None;

    for attempt in 0..=max_attempts {
        // Check timeout
        let timeout_dur = timeout_5min();
        if start_time.elapsed() > timeout_dur {
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: ETH_RPC_INTERNAL_ERROR,
                    message: GasBumpError::Timeout(timeout_dur.as_secs())
                        .to_string()
                        .into(),
                    data: None,
                }),
            ));
        }

        let estimated_gas = estimate_gas_with_timeout::<P, D, F>(&build_tx).await?;

        let gas_limit = calculate_gas_limit_with_cap(estimated_gas, attempt)?;

        let gas_price = provider.get_gas_price().await?;
        // let tx_builder = build_tx().gas(gas_limit).gas_price(gas_price).legacy();
        let tx_builder = build_tx().gas(gas_limit).gas_price(gas_price);

        debug!(
            "Sending transaction attempt {} with estimated_gas {} and gas_limit {}",
            attempt + 1,
            estimated_gas,
            gas_limit
        );

        let current_receipt = send_transaction(tx_builder).await?;

        let should_retry = !current_receipt.status()
            && attempt < max_attempts
            && likely_oog(&current_receipt, gas_limit);

        if should_retry {
            warn!(
                "Transaction failed with OOG, retrying with higher gas. Attempt {}/{}",
                attempt + 1,
                max_attempts + 1
            );
            continue;
        }

        // Store the receipt for return
        receipt = Some(current_receipt);

        check_receipt(provider, &mut receipt, attempt).await;

        break;
    }

    // Safe unwrap since we know receipt is Some at this point
    // (we break out of the loop only after assigning it)
    match receipt {
        Some(r) => Ok(r),
        None => Err(alloy_contract::Error::TransportError(
            alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                code: ETH_RPC_INTERNAL_ERROR,
                message: "No receipt available after transaction attempts"
                    .to_string()
                    .into(),
                data: None,
            }),
        )),
    }
}

fn timeout_30sec() -> Duration {
    Duration::from_secs(30)
}

fn timeout_1min() -> Duration {
    Duration::from_secs(60)
}

fn timeout_2min() -> Duration {
    Duration::from_secs(120)
}

fn timeout_5min() -> Duration {
    Duration::from_secs(300)
}

async fn check_receipt<P: Provider>(
    provider: &P,
    receipt: &mut Option<TransactionReceipt>,
    attempt: u8,
) {
    if let Some(receipt_ref) = receipt {
        if receipt_ref.status() {
            debug!(
                "Transaction succeeded after {} attempts: {:?}",
                attempt + 1,
                receipt_ref
            );
        } else {
            // Enhanced error reporting
            let trace_result = timeout(
                timeout_30sec(),
                debug_trace_tx(provider, receipt_ref.transaction_hash().to_string()),
            )
            .await;

            error!(
                "Transaction failed after {} attempts: {:?} - Trace: {:?}",
                attempt + 1,
                receipt_ref,
                trace_result
            );
        }
    }
}

async fn send_transaction<P, D>(
    tx_builder: CallBuilder<P, PhantomData<D>>,
) -> alloy_contract::Result<TransactionReceipt>
where
    P: Provider,
    D: SolCall,
{
    // Send transaction with timeout
    let pending_tx = match timeout(timeout_1min(), tx_builder.send()).await {
        Ok(result) => result?,
        Err(_) => {
            error!("Transaction send timeout");
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: ETH_RPC_INTERNAL_ERROR,
                    message: "Transaction send timeout".to_string().into(),
                    data: None,
                }),
            ));
        }
    };

    // Get receipt with timeout
    let receipt_result = timeout(timeout_2min(), pending_tx.get_receipt()).await;

    let current_receipt = match receipt_result {
        Ok(Ok(rec)) => rec,
        Ok(Err(e)) => {
            error!("Failed to get receipt: {:?}", e);
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: ETH_RPC_INTERNAL_ERROR,
                    message: format!("Failed to get receipt: {:?}", e).into(),
                    data: None,
                }),
            ));
        }
        Err(_) => {
            error!("Receipt timeout");
            return Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: ETH_RPC_INTERNAL_ERROR,
                    message: "Receipt timeout".to_string().into(),
                    data: None,
                }),
            ));
        }
    };

    Ok(current_receipt)
}

async fn estimate_gas_with_timeout<P, D, F>(build_tx: &F) -> alloy_contract::Result<u64>
where
    P: Provider,
    D: SolCall,
    F: Fn() -> SolCallBuilder<P, D>,
{
    let timeout_dur = timeout_30sec();
    match timeout(timeout_dur, build_tx().estimate_gas()).await {
        Ok(Ok(gas)) => Ok(gas),
        Ok(Err(e)) => {
            error!("Gas estimation failed: {:?}", e);
            Err(e)
        }
        Err(_elapsed) => {
            let timeout_seconds = timeout_dur.as_secs();
            error!("Gas estimation timeout after {} seconds", timeout_seconds);
            Err(alloy_contract::Error::TransportError(
                alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                    code: ETH_RPC_TIMEOUT,
                    message: format!("Gas estimation timeout after {}s", timeout_seconds).into(),
                    data: None,
                }),
            ))
        }
    }
}

fn calculate_gas_limit_with_cap(estimated_gas: u64, attempt: u8) -> alloy_contract::Result<u64> {
    let gas_limit = bumped_gas(estimated_gas, attempt);

    if gas_limit > MAX_GAS_LIMIT {
        error!(
            "Gas limit {} exceeds maximum allowed {}",
            gas_limit, MAX_GAS_LIMIT
        );
        return Err(alloy_contract::Error::TransportError(
            alloy_json_rpc::RpcError::ErrorResp(alloy_json_rpc::ErrorPayload {
                code: ETH_RPC_INVALID_PARAMS,
                message: GasBumpError::MaxGasLimitExceeded(gas_limit, MAX_GAS_LIMIT)
                    .to_string()
                    .into(),
                data: None,
            }),
        ));
    }

    Ok(gas_limit)
}

// Enhanced gas bumping with constants and better documentation
fn bumped_gas(estimated: u64, attempt: u8) -> u64 {
    // Base headroom: 64/63 to undo the 63/64 rule + 10% for proxy prelude and variance
    // = ~1.117. Use 1.20 to be safe and round up.
    let base = estimated
        .saturating_mul(BASE_GAS_HEADROOM_PERCENT)
        .saturating_div(100);

    // Per-attempt bump (compounded)
    let mut bumped = base;
    for _ in 0..attempt {
        bumped = bumped
            .saturating_mul(PER_ATTEMPT_BUMP_PERCENT)
            .saturating_div(100);
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
    let oog_margin = gas_limit
        .saturating_mul(OOG_DETECTION_MARGIN_PERCENT)
        .saturating_div(100);
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
pub mod tests {
    use super::*;
    use alloy_primitives::{Bloom, TxHash};
    use alloy_rpc_types::{Receipt, ReceiptEnvelope, ReceiptWithBloom};

    // Helper function to create a fake receipt for testing
    fn create_fake_receipt(status: bool, gas_used: u64, _gas_limit: u64) -> TransactionReceipt {
        let receipt = Receipt {
            status: status.into(),
            cumulative_gas_used: gas_used,
            logs: vec![],
        };
        let envelope = ReceiptEnvelope::Eip1559(ReceiptWithBloom {
            receipt,
            logs_bloom: Bloom::ZERO,
        });
        TransactionReceipt {
            inner: envelope,
            transaction_hash: TxHash::from([1u8; 32]),
            transaction_index: Some(0),
            block_hash: None,
            block_number: None,
            gas_used,
            effective_gas_price: 1000000000,
            blob_gas_used: None,
            blob_gas_price: None,
            from: alloy_primitives::Address::from([3u8; 20]),
            to: Some(alloy_primitives::Address::from([4u8; 20])),
            contract_address: None,
        }
    }

    #[test]
    fn test_bumped_gas_base_case() {
        // Test base case with no attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 0);

        // Should be 120% of estimated (base headroom)
        assert_eq!(result, 120000);
    }

    #[test]
    fn test_bumped_gas_with_attempts() {
        // Test with 1 attempt (10% bump on top of 20% base)
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 1);

        // Base: 100000 * 1.20 = 120000
        // Attempt 1: 120000 * 1.10 = 132000
        assert_eq!(result, 132000);
    }

    #[test]
    fn test_bumped_gas_multiple_attempts() {
        // Test with 3 attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 3);

        // Base: 100000 * 1.20 = 120000
        // Attempt 1: 120000 * 1.10 = 132000
        // Attempt 2: 132000 * 1.10 = 145200
        // Attempt 3: 145200 * 1.10 = 159720
        assert_eq!(result, 159720);
    }

    #[test]
    fn test_bumped_gas_never_below_estimated() {
        // Test edge case where calculation might go below estimated
        let estimated = 1000u64;
        let result = bumped_gas(estimated, 0);

        // Should never be below estimated
        assert!(result >= estimated);
    }

    #[test]
    fn test_bumped_gas_overflow_protection() {
        // Test with very large numbers to ensure no overflow
        let estimated = u64::MAX;
        let result = bumped_gas(estimated, 10);

        // Should not panic and should handle overflow gracefully
        assert!(result >= estimated);
    }

    #[test]
    fn test_likely_oog_success() {
        let gas_limit = 200000u64;
        let gas_used = 150000u64; // Well below limit
        let receipt = create_fake_receipt(true, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    #[test]
    fn test_likely_oog_failure() {
        let gas_limit = 200000u64;
        let gas_used = 195000u64; // Close to limit (within 5% margin)
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(result); // Should be OOG
    }

    #[test]
    fn test_likely_oog_margin_calculation() {
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000
        let gas_used = gas_limit - oog_margin + 1; // Just above margin
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(result); // Should be OOG
    }

    #[test]
    fn test_likely_oog_below_margin() {
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000
        let gas_used = gas_limit - oog_margin - 1; // Just below margin
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    #[test]
    fn test_likely_oog_successful_transaction() {
        // Successful transactions should never be considered OOG
        let gas_limit = 200000u64;
        let gas_used = 200000u64; // Used all gas but succeeded
        let receipt = create_fake_receipt(true, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    #[test]
    fn test_gas_bump_math_consistency() {
        // Test that gas bumping math is consistent across different scenarios
        let test_cases = vec![
            (1000u64, 0u8, 1200u64),
            (1000u64, 1u8, 1320u64),
            (1000u64, 2u8, 1452u64),
            (1000u64, 3u8, 1597u64),
            (50000u64, 0u8, 60000u64),
            (50000u64, 1u8, 66000u64),
        ];

        for (estimated, attempts, expected) in test_cases {
            let result = bumped_gas(estimated, attempts);
            assert_eq!(
                result, expected,
                "Failed for estimated={}, attempts={}",
                estimated, attempts
            );
        }
    }

    #[test]
    fn test_edge_case_zero_estimated() {
        // Test edge case with zero estimated gas
        let result = bumped_gas(0, 0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_edge_case_max_attempts() {
        // Test with maximum u8 attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, u8::MAX);

        // Should not panic and should be >= estimated
        assert!(result >= estimated);
    }

    #[test]
    fn test_oog_detection_edge_cases() {
        // Test OOG detection with edge cases
        let gas_limit = 100000u64;

        // Exactly at the margin
        let gas_used = gas_limit - (gas_limit / 20);
        let receipt = create_fake_receipt(false, gas_used, gas_limit);
        assert!(likely_oog(&receipt, gas_limit));

        // Just below the margin
        let gas_used = gas_limit - (gas_limit / 20) - 1;
        let receipt = create_fake_receipt(false, gas_used, gas_limit);
        assert!(!likely_oog(&receipt, gas_limit));
    }

    #[test]
    fn test_gas_bump_algorithm_correctness() {
        // Test that the gas bumping algorithm produces expected results
        let test_cases = vec![
            // (estimated, attempt, expected)
            (100000u64, 0u8, 120000u64), // Base case: 20% headroom
            (100000u64, 1u8, 132000u64), // First retry: 20% + 10% = 32%
            (100000u64, 2u8, 145200u64), // Second retry: 20% + 10% + 10% = 45.2%
            (100000u64, 3u8, 159720u64), // Third retry: 20% + 10% + 10% + 10% = 59.72%
            (50000u64, 0u8, 60000u64),   // Different base amount
            (50000u64, 1u8, 66000u64),   // Different base amount with retry
        ];

        for (estimated, attempt, expected) in test_cases {
            let result = bumped_gas(estimated, attempt);
            assert_eq!(
                result, expected,
                "Gas bump failed for estimated={}, attempt={}, expected={}, got={}",
                estimated, attempt, expected, result
            );
        }
    }

    #[test]
    fn test_gas_bump_never_below_estimated() {
        // Ensure gas bump never goes below estimated, even with edge cases
        let test_cases = vec![
            (0u64, 0u8),
            (1u64, 0u8),
            (1u64, 1u8),
            (1u64, 10u8),
            (u64::MAX, 0u8),
            (u64::MAX, 1u8),
        ];

        for (estimated, attempt) in test_cases {
            let result = bumped_gas(estimated, attempt);
            assert!(
                result >= estimated,
                "Gas bump went below estimated: estimated={}, attempt={}, result={}",
                estimated,
                attempt,
                result
            );
        }
    }

    #[test]
    fn test_oog_detection_accuracy() {
        // Test OOG detection accuracy with various scenarios
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000

        // Test cases: (gas_used, status, expected_oog)
        let test_cases = vec![
            // Successful transactions should never be OOG
            (gas_limit, true, false),
            (gas_limit - 1, true, false),
            (gas_limit - oog_margin, true, false),
            // Failed transactions near the limit should be OOG
            (gas_limit, false, true),
            (gas_limit - 1, false, true),
            (gas_limit - oog_margin + 1, false, true),
            // Failed transactions well below limit should not be OOG
            (gas_limit - oog_margin - 1, false, false),
            (gas_limit / 2, false, false),
        ];

        for (gas_used, status, expected_oog) in test_cases {
            let receipt = create_fake_receipt(status, gas_used, gas_limit);
            let result = likely_oog(&receipt, gas_limit);
            assert_eq!(
                result, expected_oog,
                "OOG detection failed: gas_used={}, status={}, expected={}, got={}",
                gas_used, status, expected_oog, result
            );
        }
    }

    #[test]
    fn test_gas_bump_overflow_protection() {
        // Test that gas bumping handles overflow gracefully
        let large_estimated = u64::MAX / 2; // Large but not max to allow for multiplication

        // Test with various attempt counts
        for attempt in 0..=10 {
            let result = bumped_gas(large_estimated, attempt);
            // Should not panic and should be >= estimated
            assert!(
                result >= large_estimated,
                "Gas bump overflow protection failed: estimated={}, attempt={}, result={}",
                large_estimated,
                attempt,
                result
            );
        }
    }

    #[test]
    fn test_gas_bump_mathematical_properties() {
        // Test mathematical properties of gas bumping
        let base_estimated = 100000u64;

        // Property 1: Each attempt should increase gas (when not at overflow)
        let mut prev_gas = bumped_gas(base_estimated, 0);
        for attempt in 1..=5 {
            let current_gas = bumped_gas(base_estimated, attempt);
            assert!(
                current_gas >= prev_gas,
                "Gas should not decrease between attempts: attempt={}, prev={}, current={}",
                attempt,
                prev_gas,
                current_gas
            );
            prev_gas = current_gas;
        }

        // Property 2: Gas bump should be monotonically increasing with attempts
        for attempt in 0..=10 {
            let gas = bumped_gas(base_estimated, attempt);
            assert!(
                gas >= base_estimated,
                "Gas should never be below estimated: attempt={}, estimated={}, gas={}",
                attempt,
                base_estimated,
                gas
            );
        }
    }

    // Helper function for generating contract revert errors in tests
    pub fn generate_contract_revert_error<T: alloy_sol_types::SolInterface>(
        input: T,
    ) -> alloy_contract::Error {
        use alloy_contract::Error::TransportError;
        use alloy_json_rpc::ErrorPayload;
        use alloy_json_rpc::RpcError::ErrorResp;

        const CONTRACT_ERROR_TEMPLATE: &str =
            r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

        let error = CONTRACT_ERROR_TEMPLATE.replace(
            "<to_replace>",
            &format!("0x{}", hex::encode(input.abi_encode())),
        );
        let payload = serde_json::from_str::<ErrorPayload>(&error).unwrap();
        TransportError(ErrorResp(payload))
    }
}
