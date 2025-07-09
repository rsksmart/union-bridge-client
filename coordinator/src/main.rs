use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::BitVmxBrokerClient;
use common::runtime_sync::RuntimeSync;
use common::{msg_broker::broker::BrokerClient, shutdown_flag::ShutdownFlag};
use coordinator::{
    config::{Config, Logger},
    coordinator::Coordinator,
    monitor::Monitor,
};
use log::{error, info};
use std::sync::Arc;
use transaction_dispatcher::config::ConfigAsLib;

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
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_path = matches.get_one::<String>(CONFIG_CLI_FLAG);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: ConfigAsLib =
        ConfigAsLib::load(config_path).expect("Failed to load transaction dispatcher config");

    let block_broker = BrokerClient::new(
        config.block_broker.ip,
        config.block_broker.port,
        config.broker_client_id,
    );
    let log_broker = BrokerClient::new(
        config.log_broker.ip,
        config.log_broker.port,
        config.broker_client_id,
    );
    let bitvmx_broker = Arc::new(BitVmxBrokerClient::new(
        config.bitvmx_broker.ip,
        config.bitvmx_broker.port,
        config.broker_client_id,
    );

    let monitor = Monitor::new(
        log_broker,
        block_broker,
        bitvmx_broker.clone(),
        config.get_peg_manager_contract_addresses(),
    );

    let shutdown_flag = ShutdownFlag::init();

    let rt_sync = RuntimeSync::new().context("Failed to create runtime sync")?;

    let contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib(
        rt_sync.clone(),
        tx_dispatcher_config,
    )?;

    let mut coordinator = Coordinator::new(
        rt_sync,
        monitor,
        contracts_gateway,
        bitvmx_broker,
        shutdown_flag.clone(),
    );
    coordinator.run().inspect_err(|e| {
        error!("Unrecoverable error running coordinator: {:?}", e);
        // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Shutting down!");

    Ok(())
}
