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

pub async fn get_contracts_gateway<P: Provider + Clone>(
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
    .await
    .context("Could not instantiate RskContractsGateway")
}

pub fn get_contracts_gateway_as_lib_sync(
    rt_sync: RuntimeSync,
    config: config::ConfigAsLib,
) -> Result<RskContractsGateway<impl Provider + Clone>, DomainErrors> {
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
        //TODO (JIRA) https://rsklabs.atlassian.net/browse/UB-318 to be removed when no op accounts would be used from the user-api
        .with_simple_nonce_management()
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
    .await
    .map_err(|e| DomainErrors::InternalServerError(format!("Failed to create gateway: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::runtime_sync::RuntimeSync;

    #[test]
    fn test_get_contracts_gateway_as_lib_sync_returns_domain_errors() {
        // This test verifies that RuntimeSync properly propagates DomainErrors
        // without shadowing them as anyhow::Error

        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        // Create a future that returns DomainErrors
        let test_future = async {
            Err::<(), DomainErrors>(DomainErrors::InternalServerError(
                "Test error propagation".to_string(),
            ))
        };

        let result: Result<(), DomainErrors> = rt_sync.run(test_future);

        // Verify the error type is preserved
        match result {
            Err(DomainErrors::InternalServerError(msg)) => {
                assert_eq!(msg, "Test error propagation");
            }
            _ => panic!("Expected DomainErrors::InternalServerError"),
        }
    }

    #[test]
    fn test_runtime_sync_preserves_different_domain_error_variants() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        // Test InvalidAddress variant
        let result: Result<(), DomainErrors> =
            rt_sync.run(async { Err(DomainErrors::InvalidAddress("0x123".to_string())) });

        match result {
            Err(DomainErrors::InvalidAddress(addr)) => {
                assert_eq!(addr, "0x123");
            }
            _ => panic!("Expected DomainErrors::InvalidAddress"),
        }

        // Test PeginAlreadyRequested variant
        let result: Result<(), DomainErrors> =
            rt_sync.run(async { Err(DomainErrors::PeginAlreadyRequested("tx123".to_string())) });

        match result {
            Err(DomainErrors::PeginAlreadyRequested(tx)) => {
                assert_eq!(tx, "tx123");
            }
            _ => panic!("Expected DomainErrors::PeginAlreadyRequested"),
        }
    }

    #[test]
    fn test_runtime_sync_preserves_success_with_complex_types() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        // Test with a success case
        let result: Result<String, DomainErrors> = rt_sync.run(async { Ok("success".to_string()) });

        match result {
            Ok(val) => assert_eq!(val, "success"),
            Err(_) => panic!("Expected Ok result"),
        }
    }
}
