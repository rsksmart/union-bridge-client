use crate::contracts::{
    bitcoin_manager, committee_registry, peg_manager, signature_manager, stream_manager,
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
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

// TODO(Jira): properly test this, creating a mockeable wrapper around SolCallBuilder - https://rsklabs.atlassian.net/browse/UB-109
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
    let mut receipt;
    let mut attempt = 0;
    loop {
        // this works also as an eth_call that would check error types, etc., if not do a manual .call()
        let estimated_gas = build_tx().estimate_gas().await?;
        let gas_limit = bumped_gas(estimated_gas, attempt);

        let tx_builder = build_tx().gas(gas_limit);

        debug!(
            "Sending transaction with estimated_gas {estimated_gas}: {:?}",
            tx_builder
        );

        receipt = tx_builder.send().await?.get_receipt().await?;

        let should_retry =
            !receipt.status() && attempt < max_attempts && likely_oog(&receipt, gas_limit);
        if should_retry {
            warn!("Bumping transaction gas");
            attempt += 1;
            continue;
        }

        if receipt.status() {
            debug!("Transaction succeeded: {:?}", receipt);
        } else {
            let trace_result =
                debug_trace_tx(provider, receipt.transaction_hash().to_string()).await?;
            error!(
                "Transaction failed: {:?} - {:?} - {:?}",
                receipt, tx_builder, trace_result
            );
        }

        break;
    }

    Ok(receipt)
}

// helper: apply headroom for proxies and add per-attempt bump
fn bumped_gas(estimated: u64, attempt: u8) -> u64 {
    // base headroom: 64/63 to undo the 63/64 rule + 10% for proxy prelude and variance
    // = ~1.117. Use 1.20 to be safe and round up.

    let base = estimated.saturating_mul(120).saturating_div(100);

    // per-attempt 10% bump (compounded)
    let mut bumped = base;
    for _ in 0..attempt {
        bumped = bumped.saturating_mul(110).saturating_div(100);
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

fn likely_oog(receipt: &TransactionReceipt, gas_limit: u64) -> bool {
    let oog_margin = gas_limit / 50; // 2% margin
    let oog_candidate =
        !receipt.status() && receipt.gas_used() >= gas_limit.saturating_sub(oog_margin);

    if oog_candidate {
        warn!(
            "Gas used: {}, Gas limit: {}, OOG margin: {}",
            receipt.gas_used(),
            gas_limit,
            oog_margin
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
            .unwrap_or_else(|| DomainErrors::NoRevertError(format!("{:?}", err)))
    }
}

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
