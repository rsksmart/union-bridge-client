use crate::contracts::common::send_tx_with_gas_bump;
use alloy_primitives::{Address, TxHash};
use alloy_provider::Provider;
use common::mocks::fake_contracts::FakePegManager;
use common::mocks::fake_contracts::FakePegManager::FakePegManagerInstance;
use log::info;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait PegManagerContractApi {
    async fn notify_check_fork_completion(
        &self,
        pegout_id: &str,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;
}

// needed so we can create a PegManagerContractApi trait for tests mocking
#[derive(Clone)]
pub struct FakePegManagerContract<P: Provider> {
    contract_instance: FakePegManagerInstance<P>,
}

impl<P: Provider> FakePegManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to FakePegManagerContract @ {contract_address}");
        let contract_instance = FakePegManager::new(contract_address, provider);
        FakePegManagerContract { contract_instance }
    }
}

impl<P: Provider> PegManagerContractApi for FakePegManagerContract<P> {
    async fn notify_check_fork_completion(
        &self,
        pegout_id: &str,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || {
                self.contract_instance
                    .checkForkComplete(pegout_id.to_string())
            },
            gas_bumps,
        )
        .await
    }
}
