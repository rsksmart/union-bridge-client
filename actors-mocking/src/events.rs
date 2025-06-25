use crate::fake_contracts::FakePegManager;
use crate::fake_contracts::FakePegManager::FakePegManagerInstance;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use anyhow::{Context, Result, anyhow, bail};
use common::anvil_mocks::get_anvil_block_pow;
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::SystemTime;
use union_contracts::bindings::peg_manager::PegManager::PegManagerInstance;

pub struct Executor<P: Provider> {
    provider: P,
    #[allow(dead_code)]
    real_peg_manager: PegManagerInstance<P>, // TODO use it to call methods from CLI if we want
    fake_peg_manager: FakePegManagerInstance<P>,
}

impl<P: Provider + Clone> Executor<P> {
    pub async fn new(provider: P, provider_url: &str) -> Result<Self> {
        println!("Deploying FakePegManager to {}...", provider_url);

        // deploy FakePegManager
        let fake_peg_manager = FakePegManager::deploy(provider.clone())
            .await
            .context("Cannot deploy FakePegManager")?;

        println!(
            "FakePegManager deployed at {}...",
            fake_peg_manager.address()
        );

        let real_peg_manager = Self::deploy_real_peg_manager(&provider, provider_url)?;

        Ok(Self {
            provider,
            real_peg_manager,
            fake_peg_manager,
        })
    }

    // TODO check with Pedro if we can improve the deployment via Rust (alloy) directly, not via sh script
    fn deploy_real_peg_manager(provider: &P, rpc_url: &str) -> Result<PegManagerInstance<P>> {
        println!("Deploying real PegManager on {}...", rpc_url);

        let union_contracts_deploy_script = env::var("UNION_CONTRACTS_DEPLOY_SCRIPT")
            .context("UNION_CONTRACTS_DEPLOY_SCRIPT not set")?;

        let mut child = Command::new("bash")
            .arg(format!("{}", union_contracts_deploy_script))
            .env("RPC_URL", rpc_url)
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to start script");

        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let reader = BufReader::new(stdout);

        let mut opt_addr = None;
        for line_res in reader.lines() {
            let line = line_res.expect("Failed to read line from script output");
            println!("{}", line);
            if opt_addr.is_none() {
                opt_addr = Self::try_get_peg_manager_address(line);
            }
        }

        child.wait().expect("Failed to wait on child");

        let address = match opt_addr {
            Some(addr) => {
                println!("Real PegManager deployed at address: {}", addr);
                addr
            }
            None => {
                bail!("PegManager address not found in output");
            }
        };

        let real_peg_manager_address = address
            .parse::<Address>()
            .context("Parsing logged address to Address failed")?;

        Ok(PegManagerInstance::new(
            real_peg_manager_address,
            provider.clone(),
        ))
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

        let anvil_effort = get_anvil_block_pow().into_effort();

        let required_num_blocks: u32 = env::var("CHECK_FORK_REQUIRED_NUM_BLOCKS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(5);

        // required_num_blocks - 1 to complete req pow before req blocks
        let blocks_to_fill_effort = U256::from_be_slice(&(required_num_blocks - 1).to_be_bytes());
        let effort_alloy = U256::from_be_slice(&anvil_effort.to_big_endian());
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

    fn try_get_peg_manager_address(line: String) -> Option<String> {
        if line.contains("PegManager.sol  address") {
            // Expect format: "...PegManager.sol  address:  0x..."
            let parts: Vec<&str> = line.split("address:").collect();
            if parts.len() == 2 {
                let address = parts[1].trim().to_string();
                return Some(address);
            }
        }
        None
    }
}
