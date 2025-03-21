use anyhow::{Context, Ok, Result};
use clap::{Arg, Command};
use common::config::Config;
use log::info;
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
                .help("Sets the path to the log4rs configuration file")
                .default_value("log4rs.yaml"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory")
                .default_value("config/dev"),
        )
        .get_matches();

    let logger_path: &String = matches.get_one(LOGGER_CLI_FLAG).unwrap();
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let config_path: &String = matches.get_one(CONFIG_CLI_FLAG).unwrap();
    let config = Config::load(config_path).expect("Failed to load config");

    let store = RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))?;

    let stored_logs = store
        .get_all_logs()
        .context("Failed to retrieve logs from storage")?;

    info!("Retrieved {} logs from storage", stored_logs.len());

    let pretty_logs = serde_json::to_string_pretty(&stored_logs)
        .context("Failed to serialize logs to pretty JSON")?;
    info!("Logs:\n{}", pretty_logs);

    log::logger().flush();

    Ok(())
}
