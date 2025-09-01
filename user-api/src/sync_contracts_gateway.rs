use common::types::Address;
use std::sync::Arc;
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::{PeginAddressInput, PeginAddressOutput};

// Synchronous wrapper trait that is dyn-compatible
// NOTE: This uses a thread::spawn hack to avoid runtime nesting issues
// This should only be used in user-api where we need to call async code from axum handlers
pub trait SyncContractsGatewayApi: Send + Sync {
    fn my_address(&self) -> Address;
    fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors>;
}

// Runtime-agnostic wrapper that can work in any context
// WARNING: This uses thread::spawn as a workaround for runtime nesting issues
// This is a hack and should not be used outside of user-api
pub struct SyncContractsGateway<T> {
    gateway: Arc<T>,
}

impl<T> SyncContractsGateway<T> {
    pub fn new(gateway: T) -> Self {
        Self {
            gateway: Arc::new(gateway),
        }
    }

    pub fn from_arc(gateway: Arc<T>) -> Self {
        Self { gateway }
    }
}

impl<T: RskContractsGatewayApi + Send + Sync + 'static> SyncContractsGatewayApi
    for SyncContractsGateway<T>
{
    fn my_address(&self) -> Address {
        self.gateway.my_address()
    }

    fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors> {
        // HACK: Create a new thread-local runtime just for this operation
        // This avoids the nested runtime issue but is not ideal
        // This should only be used in user-api
        let gateway = self.gateway.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                DomainErrors::InternalServerError(format!("Failed to create runtime: {}", e))
            })?;
            rt.block_on(gateway.get_temporary_pegin_address(input))
        })
        .join()
        .map_err(|e| DomainErrors::InternalServerError(format!("Thread panic: {:?}", e)))?
    }
}
