use std::sync::Arc;

use common::types::Address;
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::{
    PeginAddressInput, PeginAddressOutput, RequestPegoutInput, RequestPegoutOutput,
};

// Synchronous wrapper trait that is dyn-compatible
// NOTE: This uses Handle::current().block_on() to call async code from sync contexts
// This should only be used in user-api where we need to call async code from axum handlers
pub trait SyncContractsGatewayApi: Send + Sync {
    fn my_address(&self) -> Address;
    fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, DomainErrors>;

    fn request_pegout(
        &self,
        input: RequestPegoutInput,
    ) -> Result<RequestPegoutOutput, DomainErrors>;
}

// Runtime-agnostic wrapper that can work in async contexts
// Uses the current runtime's Handle to execute async operations synchronously
// This should only be used in user-api where Axum handlers need sync interfaces
pub struct SyncContractsGateway<T> {
    gateway: Arc<T>,
}

impl<T> SyncContractsGateway<T> {
    pub fn new(gateway: T) -> Self {
        Self { gateway: Arc::new(gateway) }
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
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.gateway.get_temporary_pegin_address(input))
        })
    }

    fn request_pegout(
        &self,
        input: RequestPegoutInput,
    ) -> Result<RequestPegoutOutput, DomainErrors> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.gateway.request_pegout(input))
        })
    }
}
