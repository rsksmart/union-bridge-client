use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use anyhow::{Context, Result, anyhow};
use common::types::BlockPow;
use union_contracts::bindings::peg_manager::PegManager::PegManagerInstance;

use crate::fake_contracts::FakePegManager;
use crate::fake_contracts::FakePegManager::FakePegManagerInstance;

pub struct Executor<P: Provider> {
    provider: P,
    #[allow(dead_code)]
    real_peg_manager: Option<PegManagerInstance<P>>, // TODO use it to call methods from CLI if we want
    fake_peg_manager: FakePegManagerInstance<P>,
}

impl<P: Provider + Clone> Executor<P> {
    pub async fn new(provider: P, provider_url: &str) -> Result<Self> {
        println!("Deploying FakePegManager to {}...", provider_url);

        let real_peg_manager = Self::deploy_real_peg_manager(&provider)?;

        // deploy FakePegManager: must go after real PegManager deployment to not affect generated addresses
        let fake_peg_manager = FakePegManager::deploy(provider.clone())
            .await
            .context("Cannot deploy FakePegManager")?;

        println!("FakePegManager deployed at {}", fake_peg_manager.address());

        Ok(Self {
            provider,
            real_peg_manager: Some(real_peg_manager),
            fake_peg_manager,
        })
    }

    // TODO check with Pedro if we can improve the deployment via Rust (alloy) directly, not via sh script
    fn deploy_real_peg_manager(provider: &P) -> Result<PegManagerInstance<P>> {
        println!("Deploying real PegManager");

        let contracts_path =
            env::var("UNION_CONTRACTS_PATH").context("UNION_CONTRACTS_PATH not set")?;
        let deploy_script = env::var("UNION_CONTRACTS_DEPLOY_SCRIPT")
            .context("UNION_CONTRACTS_DEPLOY_SCRIPT not set")?;
        let setup_script = env::var("UNION_CONTRACTS_SETUP_SCRIPT")
            .context("UNION_CONTRACTS_SETUP_SCRIPT not set")?;

        // Run deployment script
        let output_lines = Self::run_script(&contracts_path, &deploy_script)
            .context("Failed to execute deploy script")?;

        // Find PegManager address in deploy script output
        let address_line = output_lines
            .iter()
            .find_map(|line| Self::try_get_peg_manager_address(line.clone()))
            .ok_or_else(|| anyhow!("PegManager address not found in output"))?;

        // Run setup script
        println!("Running setup script...");
        Self::run_script(&contracts_path, &setup_script)
            .context("Failed to execute setup script")?;

        let real_peg_manager_address = address_line
            .parse::<Address>()
            .context("Parsing logged address to Address failed")?;

        println!("Real PegManager deployed at {}", real_peg_manager_address);

        Ok(PegManagerInstance::new(
            real_peg_manager_address,
            provider.clone(),
        ))
    }

    fn run_script(contracts_path: &str, script_path: &str) -> Result<Vec<String>> {
        let script_full_path = format!("{}/{}", contracts_path, script_path);

        let mut child = Command::new("bash")
            .current_dir(contracts_path)
            .arg(script_full_path)
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to spawn script: {}", script_path))?;

        let stdout = child.stdout.take().context("Failed to capture stdout")?;
        let reader = BufReader::new(stdout);

        let mut output_lines = Vec::new();
        for line_res in reader.lines() {
            let line = line_res.context("Failed to read line from script output")?;
            println!("{}", line);
            output_lines.push(line);
        }

        child.wait().context("Failed to wait for script")?;
        Ok(output_lines)
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
