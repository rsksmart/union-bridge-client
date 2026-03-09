use std::path::Path;

use alloy_provider::network::EthereumWallet;
use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use anyhow::{Context, Result};
use common::runtime_sync::RuntimeSync;
use common::types::Address;
use key_manager::key_manager::KeyManager;
use log::info;

use crate::rsk_gateway::{DomainErrors, RskContractsGateway};

pub mod config;
mod contracts;
pub mod rsk_gateway;

#[cfg(feature = "types")]
pub mod types;

#[derive(Debug)]
pub enum GatewayRole {
    User,
    Member,
}

/// Get a contracts gateway instance.
///
/// # Errors
///
/// Returns an error if the gateway cannot be instantiated.
pub async fn get_contracts_gateway<P: Provider + Clone>(
    provider: P,
    config: config::Config,
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

/// Get a contracts gateway instance synchronously with a specific role.
///
/// # Errors
///
/// Returns a `DomainErrors` if the gateway cannot be created.
pub fn get_contracts_gateway_as_lib_sync_with_role(
    rt_sync: &RuntimeSync,
    config: config::Config,
    role: GatewayRole,
) -> Result<RskContractsGateway<impl Provider + Clone + 'static>, DomainErrors> {
    let rt_sync = rt_sync.clone();
    rt_sync.run(create_contracts_gateway_impl_with_role(config, role))
}

/// Get a contracts gateway instance asynchronously with a specific role.
///
/// # Errors
///
/// Returns an error if the gateway cannot be created.
pub async fn get_contracts_gateway_as_lib(
    config: config::Config,
    role: GatewayRole,
) -> Result<RskContractsGateway<impl Provider + Clone>> {
    create_contracts_gateway_impl_with_role(config, role)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create contracts gateway: {e}"))
}

async fn create_contracts_gateway_impl_with_role(
    config: config::Config,
    role: GatewayRole,
) -> Result<RskContractsGateway<impl Provider + Clone>, DomainErrors> {
    let key_path = match role {
        GatewayRole::User => &config.tx_dispatcher_config.key_store.user_path,
        GatewayRole::Member => &config.tx_dispatcher_config.key_store.member_path,
    };

    let key_store_path = Path::new(key_path);

    info!(
        "Getting {} signer from key at {}",
        match role {
            GatewayRole::User => "user",
            GatewayRole::Member => "member",
        },
        key_store_path.display()
    );

    let signer = KeyManager::get_signer(key_store_path).map_err(|e| {
        DomainErrors::InternalServerError(format!("Failed to get {role:?} signer: {e}"))
    })?;
    info!(
        "Got {} signer with address {}",
        match role {
            GatewayRole::User => "user",
            GatewayRole::Member => "member",
        },
        signer.address()
    );

    let signer_address = signer.address().into();

    let wallet = EthereumWallet::from(signer);
    let rsk_url = &config.provider.rootstock.url;
    let ws = WsConnect::new(rsk_url);

    let provider = ProviderBuilder::new()
        //TODO (JIRA) UB-318 to be removed when no op accounts would be used from the user-api
        .with_simple_nonce_management()
        .wallet(wallet)
        .connect_ws(ws)
        .await
        .map_err(|e| {
            DomainErrors::InternalServerError(format!("Failed to connect to provider: {e}"))
        })?;

    info!(
        "Connected to Rootstock at {} as {:?} with address {}",
        &config.provider.rootstock.url, role, signer_address
    );

    RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        &config.tx_dispatcher_config.transaction,
        signer_address,
    )
    .await
    .map_err(|e| DomainErrors::InternalServerError(format!("Failed to create gateway: {e}")))
}

#[cfg(test)]
mod tests {
    use common::runtime_sync::RuntimeSync;

    use super::*;

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
            Err(e) => panic!("Expected Ok result, got {e:?}"),
        }
    }
}
