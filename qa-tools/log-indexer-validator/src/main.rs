use anyhow::{Context, Ok, Result};
use clap::{Arg, Command};
use common::types::RskLog;
use log::info;
use log_indexer::config::{Config, Logger};
use log_indexer::store::RawLogStore;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

fn main() -> Result<()> {
    let matches = Command::new("Check log-indexer tool")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_path = matches.get_one::<String>(CONFIG_CLI_FLAG);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    let store = RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))?;

    let stored_logs = get_all_logs(store)?;

    info!("Retrieved {} logs from storage", stored_logs.len());

    let pretty_logs = serde_json::to_string_pretty(&stored_logs)
        .context("Failed to serialize logs to pretty JSON")?;
    info!("Logs:\n{}", pretty_logs);

    log::logger().flush();

    Ok(())
}

#[cfg(feature = "test-utils")]
fn get_all_logs(store: RawLogStore) -> Result<Vec<RskLog>> {
    let stored_logs = store
        .get_all_logs()
        .context("Failed to retrieve logs from storage")?;
    Ok(stored_logs)
}

#[cfg(not(feature = "test-utils"))]
fn get_all_logs(_store: RawLogStore) -> Result<Vec<RskLog>> {
    panic!("Launch this tool with testing feature!")
}
