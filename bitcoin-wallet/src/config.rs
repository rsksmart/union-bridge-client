use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Network;
use serde::Deserialize;

use crate::cli::CliOpts;

#[derive(Debug, Clone)]
pub struct Config {
    pub utxo_db_path: PathBuf,
    pub sats_per_byte: Option<u64>,
    pub network: Option<Network>,
    pub private_key_wif: Option<String>,
    pub rpc_url: Option<String>,
    pub rpc_user: Option<String>,
    pub rpc_password: Option<String>,
    pub utxos: Vec<UtxoEntry>,
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
    utxos: Vec<UtxoEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
}

impl Config {
    pub fn load(cli: &CliOpts) -> Result<(Self, Option<PathBuf>)> {
        let (file_config, config_path) =
            load_file(cli.config.as_deref(), cli.config_dir.as_deref())?;
        let mut file_config = file_config;

        // Resolve utxo_db_path with precedence:
        // 1) --utxo-db flag or WALLET_UTXO_DB env (clap maps env to this opt) → use as absolute/as-is
        // 2) Otherwise, build from config
        let utxo_db_path = cli.utxo_db.clone().or_else(|| {
            Self::build_db_path_from_conf(&mut file_config)
        }).ok_or_else(|| anyhow!(
            "UTXO database path must be provided via --utxo-db (or WALLET_UTXO_DB), or set utxo_db_path in config together with BASE_STORAGE_PATH env"
        ))?;

        let sats_per_byte = cli.sats_per_byte.or(file_config.sats_per_byte);

        let network = match cli.network.as_deref().or(file_config.network.as_deref()) {
            Some(name) => Some(parse_network(name)?),
            None => None,
        };

        let private_key_wif = cli
            .private_key_wif
            .clone()
            .or(file_config.private_key_wif.take());

        let rpc_url = cli.rpc_url.clone().or(file_config.rpc_url.take());
        let rpc_user = cli.rpc_user.clone().or(file_config.rpc_user.take());
        let rpc_password = cli.rpc_password.clone().or(file_config.rpc_password.take());

        let config = Config {
            utxo_db_path,
            sats_per_byte,
            network,
            private_key_wif,
            rpc_url,
            rpc_user,
            rpc_password,
            utxos: file_config.utxos,
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

fn load_file(
    explicit_file: Option<&Path>,
    config_dir: Option<&Path>,
) -> Result<(FileConfig, Option<PathBuf>)> {
    if let Some(path) = explicit_file {
        if !path.exists() {
            bail!("config file {} not found", path.display());
        }
        let config = read_config_file(path)?;
        return Ok((config, Some(path.to_path_buf())));
    }

    let dir_path = if let Some(dir) = config_dir {
        PathBuf::from(dir)
    } else {
        let exe_path = env::current_exe().context("failed to determine executable path")?;
        let exe_dir = exe_path
            .parent()
            .context("failed to determine executable directory")?;
        let exe_config = exe_dir.join("config");
        if exe_config.exists() {
            exe_config
        } else {
            let cwd = env::current_dir().context("failed to determine current directory")?;
            cwd.join("config")
        }
    };

    if !dir_path.exists() {
        bail!("config directory {} not found", dir_path.display());
    }

    let default_path = dir_path.join("wallet.toml");
    if !default_path.exists() {
        bail!("config file {} not found", default_path.display());
    }

    let config = read_config_file(&default_path)?;
    Ok((config, Some(default_path)))
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
