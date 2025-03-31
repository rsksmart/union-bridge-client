use crate::contracts::peg_manager::SolPegManager::registerPegInRequestCall;
use alloy_contract::CallBuilder;
use alloy_primitives::hex::FromHexError;
use alloy_primitives::ruint::ParseError;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionReceipt;
use anyhow::Context;
use std::marker::PhantomData;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

pub(super) async fn send_with_gas<P: Provider>(
    provider: P,
    tx_builder: CallBuilder<(), P, PhantomData<registerPegInRequestCall>>,
    gas_to_use: u64,
) -> anyhow::Result<TransactionReceipt> {
    let gas_price = provider
        .get_gas_price()
        .await
        .context("getting gas price")?;

    let pending_tx_builder = tx_builder
        .gas_price(gas_price)
        .gas(gas_to_use)
        .send()
        .await
        .context("sending tx")?;

    let receipt = pending_tx_builder
        .get_receipt()
        .await
        .context("getting receipt")?;

    Ok(receipt)
}
