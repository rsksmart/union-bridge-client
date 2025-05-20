use alloy_eips::BlockNumberOrTag;
use alloy_primitives::Address;
use alloy_primitives::U256;
use alloy_provider::Provider;
use anyhow::{Result, anyhow};
use common::fake_contracts::FakePegManager;
use common::fake_contracts::FakePegManager::FakePegManagerInstance;

pub struct Executor<P: Provider> {
    provider: P,
    address: Address,
}

impl<P: Provider> Executor<P> {
    pub async fn new(provider: P) -> Result<Self> {
        let address = Self::deploy(&provider)
            .await
            .expect("Cannot deploy contract");
        Ok(Self { provider, address })
    }

    async fn deploy(provider: &P) -> Result<Address> {
        let contract = FakePegManager::deploy(provider).await?;
        let bb = provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .expect("no best block");
        println!(
            "Deployed contract @ address {} and block {}",
            contract.address(),
            bb.header.hash.to_string()
        );
        Ok(*contract.address())
    }

    pub async fn request_advance_funds(&self) -> Result<()> {
        let contract = self.get_contract_instance(self.address);

        // naive way to generate different pegout id
        let bb = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .expect("no best block");

        let pegout_id = format!("pegout_{}", bb.header.number);

        let receipt = contract
            .requestAdvanceFunds(pegout_id.clone(), bb.header.number, 1000)
            .send()
            .await?
            .get_receipt()
            .await?;

        if receipt.status() {
            println!(
                "Transaction succeeded: {:?}, pegout_id: {}",
                receipt, pegout_id
            );
            Ok(())
        } else {
            eprintln!("Transaction failed: {:?}", receipt);
            Err(anyhow!("Transaction failed"))
        }
    }

    pub async fn kickoff_advance_funds(&self, pegout_id: String) -> Result<()> {
        let contract = self.get_contract_instance(self.address);

        let bb = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .expect("no best block");

        // TODO: receive from param
        let required_pow = U256::from(10_000_000_000_000u64);

        let receipt = contract
            .kickoffAdvanceFunds(pegout_id.clone(), bb.header.number, required_pow)
            .send()
            .await?
            .get_receipt()
            .await?;

        if receipt.status() {
            println!(
                "Transaction succeeded: {:?}, pegout_id: {}",
                receipt, pegout_id
            );
            Ok(())
        } else {
            eprintln!("Transaction failed: {:?}", receipt);
            Err(anyhow!("Transaction failed"))
        }
    }

    fn get_contract_instance(&self, address: Address) -> FakePegManagerInstance<(), &P> {
        FakePegManagerInstance::new(address, &self.provider)
    }
}
