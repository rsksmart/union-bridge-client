use crate::contracts::peg_manager::SolPegManager::registerPegInRequestCall;
use alloy_contract::CallBuilder;
use alloy_primitives::hex::FromHexError;
use alloy_primitives::ruint::ParseError;
use alloy_provider::Provider;
use alloy_provider::network::ReceiptResponse;
use anyhow::Context;
use common::types::{BlockHash, BlockNumber};
use std::marker::PhantomData;
use std::ops::Deref;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ParseFieldError {
    #[error("Failed to parse: {0}")]
    ParseNum(#[from] ParseError),

    #[error("Failed to parse hex: {0}")]
    ParseHex(#[from] FromHexError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContractInvokeReceipt {
    pub(crate) block_number: BlockNumber,
    pub(crate) block_hash: BlockHash,
    pub(crate) transaction_hash: String, // TODO create type
    pub(crate) gas_used: u64,
    pub(crate) status: bool,
}

pub(super) async fn send_with_gas<P: Provider>(
    provider: P,
    tx_builder: CallBuilder<(), P, PhantomData<registerPegInRequestCall>>,
    gas_to_use: u64,
) -> anyhow::Result<ContractInvokeReceipt> {
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

    Ok(ContractInvokeReceipt {
        block_number: receipt.block_number().unwrap_or_default().try_into()?,
        block_hash: receipt
            .block_hash()
            .unwrap_or_default()
            .to_string()
            .deref()
            .try_into()?,
        transaction_hash: receipt.transaction_hash().to_string(),
        gas_used: receipt.gas_used(),
        status: receipt.status(),
    })
}
