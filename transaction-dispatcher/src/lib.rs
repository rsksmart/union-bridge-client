use crate::rsk_gateway::RskContractsGateway;
use alloy_provider::network::EthereumWallet;
use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use anyhow::{Context, Result};
use common::runtime_sync::RuntimeSync;
use key_manager::key_manager::KeyManager;
use log::info;
use std::path::Path;

pub mod config;
mod contracts;
pub mod rsk_gateway;
pub mod server;
pub mod types;

pub fn get_contracts_gateway<P: Provider + Clone>(
    provider: P,
    config: config::ConfigAsBin,
) -> Result<RskContractsGateway<P>> {
    RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        config.transaction(),
    )
    .context("Could not instantiate RskContractsGateway")
}

pub fn get_contracts_gateway_as_lib(
    rt_sync: RuntimeSync,
    config: config::ConfigAsLib,
) -> Result<RskContractsGateway<impl Provider + Clone>> {
    let key_store_path = Path::new(&config.key_store.path);

    let signer = KeyManager::get_signer(key_store_path)?;
    let wallet = EthereumWallet::from(signer);
    let rsk_url = &config.provider.rootstock.url;
    let ws = WsConnect::new(rsk_url);

    let provider = rt_sync.run(async { ProviderBuilder::new().wallet(wallet).on_ws(ws).await })?;

    info!(
        "Connected to Rootstock at {} with address {}",
        &config.provider.rootstock.url, rsk_url
    );

    RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        &config.transaction,
    )
    .context("Could not instantiate RskContractsGateway")
}
