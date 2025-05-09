use anyhow::Result;
use clap::{Arg, Command};
use common::config::CommonConfig;
use common::msg_broker::broker::BrokerClient;
use common::shutdown_flag::ShutdownFlag;
use coordinator::coordinator::Coordinator;
use coordinator::monitor::Monitor;
use log::{error, info};

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");
const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
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
    CommonConfig::init_logger(logger_cfg_path, CARGO_PKG_NAME).expect("Failed to load logger");

    let block_broker = BrokerClient::new(12345); // TODO(iago) change to config
    let log_broker = BrokerClient::new(56789); // TODO(iago) change to config
    let monitor = Monitor::new(block_broker, log_broker);

    let shutdown_flag = ShutdownFlag::init();

    let mut coordinator = Coordinator::new(monitor, shutdown_flag.clone());
    coordinator.run().inspect_err(|e| {
        error!("Unrecoverable error running coordinator: {:?}", e);
        // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Shutting down!");

    Ok(())
}
