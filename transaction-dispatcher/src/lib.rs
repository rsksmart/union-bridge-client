use crate::rsk_gateway::DomainErrors;
use crate::rsk_gateway::RskContractsGateway;
use alloy_provider::network::EthereumWallet;
use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use anyhow::{Context, Result};
use common::runtime_sync::RuntimeSync;
use common::types::Address;
use key_manager::key_manager::KeyManager;
use log::info;
use std::path::Path;

pub mod config;
mod contracts;
pub mod rsk_gateway;

#[cfg(feature = "types")]
pub mod types;

pub fn get_contracts_gateway<P: Provider + Clone>(
    provider: P,
    config: config::ConfigAsBin,
    member_address: Address,
) -> Result<RskContractsGateway<P>> {
    RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        config.transaction(),
        member_address,
    )
    .context("Could not instantiate RskContractsGateway")
}

pub fn get_contracts_gateway_as_lib_sync(
    rt_sync: RuntimeSync,
    config: config::ConfigAsLib,
) -> Result<RskContractsGateway<impl Provider + Clone>> {
    rt_sync.run(create_contracts_gateway_impl(config))
}

pub async fn get_contracts_gateway_as_lib(
    config: config::ConfigAsLib,
) -> Result<RskContractsGateway<impl Provider + Clone>> {
    create_contracts_gateway_impl(config)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create contracts gateway: {}", e))
}

async fn create_contracts_gateway_impl(
    config: config::ConfigAsLib,
) -> Result<RskContractsGateway<impl Provider + Clone>, DomainErrors> {
    let key_store_path = Path::new(&config.key_store.path);

    info!("Getting signer from key at {}", key_store_path.display());
    let signer = KeyManager::get_signer(key_store_path)
        .map_err(|e| DomainErrors::InternalServerError(format!("Failed to get signer: {}", e)))?;
    info!("Got signer with address {}", signer.address());

    let signer_address = signer.address().into();

    let wallet = EthereumWallet::from(signer);
    let rsk_url = &config.provider.rootstock.url;
    let ws = WsConnect::new(rsk_url);

    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_ws(ws)
        .await
        .map_err(|e| {
            DomainErrors::InternalServerError(format!("Failed to connect to provider: {}", e))
        })?;

    info!(
        "Connected to Rootstock at {}",
        &config.provider.rootstock.url
    );

    RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        &config.transaction,
        signer_address,
    )
    .map_err(|e| DomainErrors::InternalServerError(format!("Failed to create gateway: {}", e)))
}
