use std::path::PathBuf;

use bitcoin::Network;
use ub_wallet::cli::CliOpts;
use ub_wallet::config::Config;

#[test]
fn load_config_from_directory() {
    let mut opts = CliOpts::default();
    opts.config = Some("regtest".to_string());
    opts.db_path = Some(PathBuf::from("custom-utxo-db"));

    let (config, _path) = Config::load(&opts).expect("load config");
    assert_eq!(config.network, Some(Network::Regtest));
    assert_eq!(config.sats_per_byte, Some(5));
    assert_eq!(config.rpc_user.as_deref(), Some("foo"));
    assert_eq!(config.db_path, PathBuf::from("custom-utxo-db"));
}

#[test]
fn load_config_missing_errors() {
    let mut opts = CliOpts::default();
    opts.config = Some("nonexistent".to_string());
    let err = Config::load(&opts).expect_err("missing config dir");
    let msg = err.to_string();
    assert!(msg.contains("not found"));
}
