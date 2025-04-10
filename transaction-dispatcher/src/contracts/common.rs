use alloy_contract::SolCallBuilder;
use alloy_primitives::hex::FromHexError;
use alloy_primitives::ruint::ParseError;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::SolCall;
use log::{debug, error, warn};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

// TODO(Jira): properly test this, creating a mockeable wrapper around SolCallBuilder - https://rsklabs.atlassian.net/browse/UB-109
pub(super) async fn send_tx_with_gas_bump<P, F, T>(
    build_tx: F,
    max_attempts: u8,
) -> alloy_contract::Result<TransactionReceipt>
where
    P: Provider,
    T: SolCall,
    F: Fn() -> SolCallBuilder<(), P, T>,
{
    // this works also as an eth_call, if not do a manual .call()
    let estimated_gas = build_tx().estimate_gas().await?;

    let mut receipt;
    let mut attempt = 0;
    loop {
        let attempt_increment = 1 + (0.1 * (attempt + 1) as f64) as u64;
        let gas_limit = estimated_gas * attempt_increment;

        let tx_builder = build_tx().gas(gas_limit);

        debug!("Sending transaction: {:?}", tx_builder);

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
            error!("Transaction failed: {:?} - {:?}", receipt, tx_builder);
        }

        break;
    }

    Ok(receipt)
}

fn likely_oog(receipt: &TransactionReceipt, gas_limit: u64) -> bool {
    let oog_margin = gas_limit / 100;
    !receipt.status() && receipt.gas_used >= gas_limit.saturating_sub(oog_margin)
}

#[cfg(test)]
pub(crate) mod tests {
    use alloy_json_rpc::ErrorPayload;
    use alloy_sol_types::{SolInterface, SolValue};

    pub(crate) const CONTRACT_ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

    #[allow(dead_code)]
    pub(crate) const NO_REVERT_ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"<to_replace_message>:","data":"<to_replace_data>"}"#;

    pub(crate) fn generate_contract_revert_error<T: SolInterface>(input: T) -> ErrorPayload {
        let error = CONTRACT_ERROR_TEMPLATE.replace(
            "<to_replace>",
            &format!("0x{}", hex::encode(input.abi_encode())),
        );
        serde_json::from_str::<ErrorPayload>(&error).unwrap()
    }

    #[allow(dead_code)]
    pub(crate) fn generate_no_revert_error(msg: &str, data: &str) -> ErrorPayload {
        let error = CONTRACT_ERROR_TEMPLATE
            .replace(
                "<to_replace_message>",
                &format!("0x{}", hex::encode(msg.abi_encode())),
            )
            .replace(
                "<to_replace_data>",
                &format!("0x{}", hex::encode(data.abi_encode())),
            );
        serde_json::from_str::<ErrorPayload>(&error).unwrap()
    }
}
