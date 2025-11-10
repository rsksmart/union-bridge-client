//! wallet setup functionality for multi-client deployments
//!
//! this module handles creating and funding wallets for multi-client setups.
//! each client gets two wallets:
//! - multi-client-N-member: for committee member operations
//! - multi-client-N-user: for user/transaction operations

use anyhow::{bail, Context, Result};
use key_manager::key_manager::KeyManager;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{validate_1_10, WalletAction};

/// handles wallet setup based on the requested action
pub fn handle_wallet_setup(action: &WalletAction, base_storage_path: &str) -> Result<()> {
    match action {
        WalletAction::Create { num_wallets } => {
            validate_1_10(*num_wallets, "num-wallets")?;
            setup_wallets_create(*num_wallets, base_storage_path)?;
            print_wallet_summary("create", *num_wallets);
        }
        WalletAction::Fund { num_wallets } => {
            validate_1_10(*num_wallets, "num-wallets")?;
            setup_wallets_fund(*num_wallets, base_storage_path)?;
            print_wallet_summary("fund", *num_wallets);
        }
        WalletAction::Both { num_wallets } => {
            validate_1_10(*num_wallets, "num-wallets")?;
            setup_wallets_create(*num_wallets, base_storage_path)?;
            setup_wallets_fund(*num_wallets, base_storage_path)?;
            print_wallet_summary("both", *num_wallets);
        }
    }
    Ok(())
}

fn print_wallet_summary(mode: &str, num_wallets: u8) {
    println!("\n=== wallet setup summary ===");
    println!("setup mode: {}", mode);
    println!("number of clients: {}", num_wallets);
    println!(
        "total wallets: {} (member + user per client)",
        num_wallets * 2
    );
}

fn create_or_use_keystore(keystore_path: &Path, file_name: &str, password: &str) -> Result<()> {
    let full_keystore_path = keystore_path.join(file_name);

    if full_keystore_path.exists() {
        println!(
            "[wallet-setup] key already exists at {}, skipping generation",
            full_keystore_path.display()
        );
        return Ok(());
    }

    println!(
        "[wallet-setup] creating new key at {}...",
        keystore_path.display()
    );

    // create directory if it doesn't exist
    fs::create_dir_all(keystore_path)
        .with_context(|| format!("failed to create directory {}", keystore_path.display()))?;

    // generate key using KeyManager directly
    let (generated_path, _public_key, _address) = KeyManager::generate_key(keystore_path, password)
        .context("failed to generate key with KeyManager")?;

    // rename the generated key to the desired filename
    fs::rename(&generated_path, &full_keystore_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            generated_path,
            full_keystore_path.display()
        )
    })?;

    println!(
        "[wallet-setup] key created successfully at {}",
        full_keystore_path.display()
    );

    Ok(())
}

fn derive_address_from_keystore(keystore_file: &Path, password: &str) -> Result<String> {
    if !keystore_file.exists() {
        bail!("keystore file not found: {}", keystore_file.display());
    }

    // derive address using KeyManager directly
    let (_public_key, address) = KeyManager::derive_public_key_and_address(keystore_file, password)
        .context("failed to derive address from keystore")?;

    // add 0x prefix if not present
    let address = if address.starts_with("0x") {
        address
    } else {
        format!("0x{}", address)
    };

    Ok(address)
}

fn fund_wallet(wallet_name: &str, address: &str) -> Result<()> {
    println!(
        "[fund-wallets] funding {} at address {}",
        wallet_name, address
    );

    let anvil_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    let output = Command::new("cast")
        .args([
            "send",
            "--unlocked",
            "--from",
            anvil_address,
            address,
            "--value",
            "1000000000000000000",
            "--rpc-url",
            "http://127.0.0.1:8545",
        ])
        .output()
        .context("failed to execute cast")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cast send failed: {}", stderr);
    }

    println!("[fund-wallets] successfully funded {}", wallet_name);
    std::thread::sleep(Duration::from_millis(100));

    Ok(())
}

fn setup_wallets_create(num_wallets: u8, base_storage_path: &str) -> Result<()> {
    let password = std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable is required")?;

    let keystore_base_path = PathBuf::from(base_storage_path)
        .join(".union_bridge")
        .join("keystore");

    println!("[wallet-setup] starting wallet creation...");

    for i in 1..=num_wallets {
        // create member wallet
        let member_name = format!("multi-client-{}-member", i);
        create_or_use_keystore(&keystore_base_path, &member_name, &password)
            .with_context(|| format!("failed to create member wallet for client {}", i))?;

        // create user wallet
        let user_name = format!("multi-client-{}-user", i);
        create_or_use_keystore(&keystore_base_path, &user_name, &password)
            .with_context(|| format!("failed to create user wallet for client {}", i))?;
    }

    println!("[wallet-setup] wallet creation complete! all keystores have been created.");
    Ok(())
}

fn setup_wallets_fund(num_wallets: u8, base_storage_path: &str) -> Result<()> {
    let password = std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable is required")?;

    let keystore_base_path = PathBuf::from(base_storage_path)
        .join(".union_bridge")
        .join("keystore");

    if !keystore_base_path.exists() {
        bail!(
            "keystore directory not found: {}. make sure you've created wallets first.",
            keystore_base_path.display()
        );
    }

    println!("[fund-wallets] starting to fund wallets...");
    println!(
        "[fund-wallets] using keystores from: {}",
        keystore_base_path.display()
    );

    let mut funded_count = 0;
    let mut failed_count = 0;

    for i in 1..=num_wallets {
        let (funded, failed) = fund_wallet_for_type(i, "member", &keystore_base_path, &password)?;
        funded_count += funded;
        failed_count += failed;

        let (funded, failed) = fund_wallet_for_type(i, "user", &keystore_base_path, &password)?;
        funded_count += funded;
        failed_count += failed;
    }

    println!("[fund-wallets] funding complete!");
    println!(
        "[fund-wallets] successfully funded: {} wallets",
        funded_count
    );

    if failed_count > 0 {
        println!("[fund-wallets] failed to fund: {} wallets", failed_count);
        bail!("some wallets failed to fund");
    } else {
        println!("[fund-wallets] all wallets funded successfully!");
    }

    Ok(())
}

fn fund_wallet_for_type(
    client_index: u8,
    wallet_type: &str,
    keystore_base_path: &Path,
    password: &str,
) -> Result<(usize, usize)> {
    let wallet_name = format!("multi-client-{}-{}", client_index, wallet_type);
    let wallet_path = keystore_base_path.join(&wallet_name);
    let mut funded_count = 0usize;
    let mut failed_count = 0usize;

    if wallet_path.exists() {
        match derive_address_from_keystore(&wallet_path, password) {
            Ok(address) => match fund_wallet(&wallet_name, &address) {
                Ok(_) => {
                    funded_count += 1;
                    println!("[fund-wallets] {} funded successfully", wallet_name);
                }
                Err(e) => {
                    failed_count += 1;
                    eprintln!("[fund-wallets] failed to fund {}: {}", wallet_name, e);
                }
            },
            Err(e) => {
                failed_count += 1;
                eprintln!(
                    "[fund-wallets] failed to derive address for {}: {}",
                    wallet_name, e
                );
            }
        }
    } else {
        eprintln!("[fund-wallets] wallet not found: {}", wallet_path.display());
        failed_count += 1;
    }

    println!("[fund-wallets] ---");

    Ok((funded_count, failed_count))
}
