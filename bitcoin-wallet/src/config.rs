use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Network;
use serde::Deserialize;

use crate::cli::{CliOpts, WalletMode};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Debug, Clone)]
pub struct Config {
    pub utxo_db_path: PathBuf,
    pub sats_per_byte: Option<u64>,
    pub network: Option<Network>,
    pub mode: WalletMode,
    pub private_key_wif: String,
    pub rpc_url: Option<String>,
    pub rpc_user: Option<String>,
    pub rpc_password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    network: Option<String>,
    sats_per_byte: Option<u64>,
    private_key_wif: Option<String>,
    rpc_url: Option<String>,
    rpc_user: Option<String>,
    rpc_password: Option<String>,
    utxo_db_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
}

impl Config {
    pub fn load(cli: &CliOpts) -> Result<(Self, Option<PathBuf>)> {
        let (file_config, config_path) = load_file(cli.config.as_deref())?;
        let mut file_config = file_config;

        // Resolve utxo_db_path with precedence:
        // 1) --utxo-db flag or WALLET_UTXO_DB env (clap maps env to this opt) → use as absolute/as-is
        // 2) Otherwise, build from config
        let utxo_db_path = cli
            .utxo_db
            .clone()
            .or_else(|| Self::build_db_path_from_conf(&mut file_config))
            .ok_or_else(|| anyhow!(
                "UTXO database path must be provided via --utxo-db (or WALLET_UTXO_DB), or set utxo_db_path in config together with BASE_STORAGE_PATH env"
            ))?;

        let sats_per_byte = cli.sats_per_byte.or(file_config.sats_per_byte);

        // Network is read only from the config file now
        let network = match file_config.network.as_deref() {
            Some(name) => Some(parse_network(name)?),
            None => None,
        };

        // Load the appropriate WIF based on mode
        let wif_env_var = match cli.mode {
            WalletMode::User => "USER_BITCOIN_WIF",
            WalletMode::Member => "MEMBER_BITCOIN_WIF",
        };

        let private_key_wif = env::var(wif_env_var)
            .or_else(|_| file_config.private_key_wif.take().ok_or_else(|| anyhow!("Not found in config")))
            .with_context(|| format!(
                "Private key WIF is required: set {} environment variable or define private_key_wif in config file",
                wif_env_var
            ))?;

        let rpc_url = cli.rpc_url.clone().or(file_config.rpc_url.take());
        let rpc_user = cli.rpc_user.clone().or(file_config.rpc_user.take());
        let rpc_password = cli.rpc_password.clone().or(file_config.rpc_password.take());

        // Enforce RPC configuration must be provided (via config or env mapped by clap)
        if rpc_url.is_none() || rpc_user.is_none() || rpc_password.is_none() {
            bail!(
                "RPC configuration missing: please set WALLET_RPC_URL, WALLET_RPC_USER, and WALLET_RPC_PASSWORD environment variables or define rpc_url, rpc_user, and rpc_password in the selected config file"
            );
        }

        let config = Config {
            utxo_db_path,
            sats_per_byte,
            network,
            mode: cli.mode.clone(),
            private_key_wif,
            rpc_url,
            rpc_user,
            rpc_password,
        };

        Ok((config, config_path))
    }

    // read utxo_db_path from config file and resolve under BASE_STORAGE_PATH env var
    fn build_db_path_from_conf(file_config: &FileConfig) -> Option<PathBuf> {
        file_config.utxo_db_path.as_ref().map(|rel_from_config| {
            let base = env::var("BASE_STORAGE_PATH").with_context(||
                "BASE_STORAGE_PATH environment variable must be set when using utxo_db_path from config file"
            ).unwrap();
            PathBuf::from(base).join(rel_from_config)
        })
    }
}

fn load_file(config_name: Option<&str>) -> Result<(FileConfig, Option<PathBuf>)> {
    // Resolve config directory relative to executable, then fallback to CWD
    let project_root = Path::new(CARGO_MANIFEST_DIR);
    let config_path = project_root.join("config");
    let dir_path = if config_path.exists() {
        config_path
    } else {
        let cwd = env::current_dir().context("failed to determine current directory")?;
        cwd.join("config")
    };

    if !dir_path.exists() {
        bail!("config directory {} not found", dir_path.display());
    }

    let path = if let Some(name) = config_name {
        dir_path.join(format!("{}.toml", name))
    } else {
        // Backward-compatible default
        dir_path.join("wallet.toml")
    };

    if !path.exists() {
        bail!("config file {} not found", path.display());
    }

    let config = read_config_file(&path)?;
    Ok((config, Some(path)))
}

fn read_config_file(path: &Path) -> Result<FileConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let cfg: FileConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(cfg)
}

pub fn parse_network(name: &str) -> Result<Network> {
    match name.to_lowercase().as_str() {
        "bitcoin" | "mainnet" => Ok(Network::Bitcoin),
        "testnet" | "testnet3" => Ok(Network::Testnet),
        "testnet4" => Ok(Network::Testnet4),
        "signet" => Ok(Network::Signet),
        "regtest" => Ok(Network::Regtest),
        _ => bail!("unsupported network '{name}'"),
    }
}
