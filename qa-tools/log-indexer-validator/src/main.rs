use anyhow::{Context, Ok, Result};
use clap::{Arg, Command};
use log::info;
use log_indexer::config::Config;
use log_indexer::store::RawLogStore;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn main() -> Result<()> {
    let matches = Command::new("Check log-indexer tool")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file")
                .default_value("log4rs.yaml"), // for local usage within the crate
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory"),
        )
        .get_matches();

    let default_logger = format!("{}/log4rs.yaml", CARGO_MANIFEST_DIR);
    let logger_path: &str = matches
        .get_one::<String>(LOGGER_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_logger);
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let default_config = format!("{}/../config/local", CARGO_MANIFEST_DIR);
    let config_path: &str = matches
        .get_one::<String>(CONFIG_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_config);
    let config: Config = Config::load(config_path).expect("Failed to load config");

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
