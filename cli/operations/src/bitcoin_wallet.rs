use std::env;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::address::Address as BitcoinAddress;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{CompressedPublicKey, Network, NetworkKind, PrivateKey};
use op_funding::derive_stream_funding_profile;
use protocol_params::{committee_member_count, prover_count, slots_per_package};

use crate::environments::*;
use crate::member_funding_info::CollectedMemberFundingInfo;
use crate::utils::run_wallet_command;

pub(crate) async fn handle_bitcoin_funding(
    environment: Environment,
    stream_id: u64,
    execute: bool,
    amount_override: Option<u64>,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    if execute && environment.is_remote() {
        bail!(
            "--execute flag is only supported for local environments (`local`/`docker`). For remote environments, please run the wallet commands manually."
        );
    }

    let funding_profile = derive_stream_funding_profile(
        stream_id,
        matches!(environment, Environment::Local | Environment::Docker),
        slots_per_package()?,
        committee_member_count()?,
        prover_count()?,
    )?;
    let amount = amount_override.unwrap_or(funding_profile.operator_fund_amount);

    let addresses = collect_addresses(&environment, member_funding_info)?;

    if addresses.is_empty() {
        bail!("no BitVMX funding addresses were discovered");
    }

    // Add a small fixed buffer so the wallet's `mine_utxo` amount still covers
    // the subsequent `send_to_address` transaction fee on regtest.
    let funding_utxo = amount
        .checked_mul(addresses.len() as u64)
        .and_then(|value| value.checked_add(10_000))
        .context("failed to compute wallet funding UTXO amount")?;

    println!();
    println!(
        "Derived stream {} funding: denomination={} protocol_funding={} speed_up_utxo={} advance_funds={} operator_fund_amount={}",
        stream_id,
        funding_profile.denomination,
        funding_profile.protocol_funding,
        funding_profile.speed_up_utxo,
        funding_profile.advance_funds,
        amount
    );

    if execute {
        println!("Executing wallet commands programmatically...");
        println!();
        execute_wallet_command(&addresses, amount)?;
    } else {
        print_instructions(&environment, &addresses, amount, funding_utxo);
    }

    Ok(())
}

fn collect_addresses(
    environment: &Environment,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<Vec<String>> {
    let endpoints = environment.user_api_endpoints()?;
    println!("Fetching member funding info from: {} ...", endpoints.join(", "));
    let mut addresses = Vec::new();
    for (endpoint, info) in member_funding_info {
        println!("{} -> BTC {} / RSK {}", endpoint, info.bitcoin_address, info.rsk_address);
        addresses.push(info.bitcoin_address.clone());
    }

    Ok(addresses)
}

fn print_instructions(env: &Environment, addresses: &[String], amount: u64, funding_utxo: u64) {
    let joined = addresses.join(",");
    println!(
        "Note: See the bitcoin-wallet README for how to start and use the CLI: ../cli/bitcoin-wallet/README.md\n"
    );

    match env {
        Environment::Remote(_) => {
            println!(
                "Run the following command in your bitcoin-wallet or wallet tooling for {}:",
                env.get_name()
            );
            println!("  send_to_address {} {}\n", joined, amount);
        }
        Environment::Docker | Environment::Local => {
            println!("Run the following commands in the bitcoin-wallet CLI (Regtest):");
            println!("1 =>    clear_db   (if you see a misaligned utxos error)");
            println!("2 =>    mine_utxo {}", funding_utxo);
            println!("3 =>    send_to_address {} {}", joined, amount);
            println!("4 =>    mine_block");
        }
    }
}

fn execute_wallet_command(addresses: &[String], amount: u64) -> Result<()> {
    let joined = addresses.join(",");
    let amount_str = amount.to_string();

    // just send to addresses - utxo mining and block mining handled externally
    let stdout = run_wallet_command(&["member", "send_to_address", &joined, &amount_str])?;
    println!("{}", stdout);

    Ok(())
}

pub(crate) fn derive_user_bitcoin_address_from_env(env: &Environment) -> Result<String> {
    let wif =
        env::var("USER_BITCOIN_WIF").context("USER_BITCOIN_WIF environment variable not set")?;
    derive_user_bitcoin_address(env, &wif)
}

fn derive_user_bitcoin_address(env: &Environment, wif: &str) -> Result<String> {
    let private_key = PrivateKey::from_wif(wif).context("failed to parse USER_BITCOIN_WIF")?;
    let network = bitcoin_network_for_environment(env, private_key.network);
    let public_key = private_key.public_key(&Secp256k1::new());
    let compressed = CompressedPublicKey::try_from(public_key)
        .map_err(|_| anyhow!("USER_BITCOIN_WIF must correspond to a compressed public key"))?;
    let address = BitcoinAddress::p2wpkh(&compressed, network);
    Ok(address.to_string())
}

fn bitcoin_network_for_environment(env: &Environment, network_kind: NetworkKind) -> Network {
    match env {
        Environment::Local | Environment::Docker => Network::Regtest,
        Environment::Remote(profile) => match network_kind {
            NetworkKind::Main => Network::Bitcoin,
            NetworkKind::Test => {
                let lower = profile.to_ascii_lowercase();
                if lower.contains("signet") {
                    Network::Signet
                } else if lower.contains("regtest") {
                    Network::Regtest
                } else {
                    Network::Testnet
                }
            }
        },
    }
}

