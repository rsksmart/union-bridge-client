use alloy_eips::BlockNumberOrTag;
use alloy_primitives::U256;
use alloy_provider::Provider;
use anyhow::{Context, Result, anyhow};
use common::mocks::fake_contracts::FakePegManager;
use common::mocks::fake_contracts::FakePegManager::FakePegManagerInstance;
use common::types::BlockPow;
use std::env;
use std::time::SystemTime;

pub struct Executor<P: Provider> {
    provider: P,
    fake_peg_manager: FakePegManagerInstance<P>,
}

impl<P: Provider + Clone> Executor<P> {
    pub async fn new(provider: P, provider_url: &str) -> Result<Self> {
        println!("Deploying FakePegManager to {}...", provider_url);

        // deploy FakePegManager: must go after real PegManager deployment to not affect generated addresses
        let fake_peg_manager = FakePegManager::deploy(provider.clone())
            .await
            .context("Cannot deploy FakePegManager")?;

        println!("FakePegManager deployed at {}", fake_peg_manager.address());

        Ok(Self {
            provider,
            fake_peg_manager,
        })
    }

    pub async fn request_advance_funds(&self) -> Result<()> {
        // naive way to generate a different pegout id each time
        let naive_id = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let pegout_id = format!("pegout_{naive_id}");

        let receipt = self
            .fake_peg_manager
            .requestAdvanceFunds(pegout_id.to_string(), 1000)
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

    pub async fn advance_funds(&self, pegout_id: String) -> Result<()> {
        let bb = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .expect("no best block");

        let required_num_blocks: u32 = env::var("CHECK_FORK_REQUIRED_NUM_BLOCKS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(5);

        // required_num_blocks - 1 to complete req pow before req blocks
        let blocks_to_fill_effort = U256::from_be_slice(&(required_num_blocks - 1).to_be_bytes());
        let effort_alloy = U256::from_be_slice(&Self::get_effort().into_effort().to_big_endian());
        let required_effort = effort_alloy
            .checked_mul(blocks_to_fill_effort)
            .expect("required_effort should not overflow");

        let utxo_id = format!("utxo_{}", bb.header.number);
        let operator_id = format!("operator_{}", bb.header.number);

        let receipt = self
            .fake_peg_manager
            .advanceFunds(
                pegout_id.clone(),
                utxo_id,
                operator_id,
                required_effort,
                required_num_blocks,
            )
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

    #[cfg(feature = "anvil")]
    fn get_effort() -> BlockPow {
        use common::anvil_mocks::get_anvil_block_pow;
        get_anvil_block_pow()
    }

    #[cfg(not(feature = "anvil"))]
    fn get_effort() -> BlockPow {
        panic!("This crate should be used with 'anvil' feature enabled");
    }
}
