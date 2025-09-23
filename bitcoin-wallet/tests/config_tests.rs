use std::fs;
use std::path::PathBuf;

use bitcoin::Network;
use tempfile::tempdir;
use ub_wallet::cli::CliOpts;
use ub_wallet::config::Config;

#[test]
fn load_config_from_directory() {
    let temp = tempdir().expect("temp dir");
    let config_dir = temp.path().join("cfg");
    fs::create_dir(&config_dir).expect("create config dir");

    let config_contents = r#"
network = "regtest"
sats_per_byte = 8
rpc_url = "http://localhost:18443"
rpc_user = "alice"
rpc_password = "secret"
utxo_db_path = "custom-utxo-db"
"#;
    fs::write(config_dir.join("wallet.toml"), config_contents).expect("write config");

    let mut opts = CliOpts::default();
    opts.config_dir = Some(config_dir.clone());

    let (config, path) = Config::load(&opts).expect("load config");
    let path = path.expect("config path");
    assert_eq!(path, config_dir.join("wallet.toml"));
    assert_eq!(config.network, Some(Network::Regtest));
    assert_eq!(config.sats_per_byte, Some(8));
    assert_eq!(config.rpc_user.as_deref(), Some("alice"));
    assert_eq!(config.utxo_db_path, PathBuf::from("custom-utxo-db"));
}

#[test]
fn load_config_missing_errors() {
    let temp = tempdir().expect("temp dir");
    let missing_dir = temp.path().join("missing");
    let mut opts = CliOpts::default();
    opts.config_dir = Some(missing_dir);
    let err = Config::load(&opts).expect_err("missing config dir");
    let msg = err.to_string();
    assert!(msg.contains("config directory"));
}
