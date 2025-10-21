use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::{
    config::CommonConfig,
    msg_broker::broker::{BITVMX_L2_BROKER_CLIENT_ID, BitVmxBrokerClient, BrokerClient},
    runtime_sync::RuntimeSync,
    shutdown_flag::ShutdownFlag,
};
use coordinator::store::CoordinatorStore;
use coordinator::{
    config::{Config, Logger},
    coordinator::Coordinator,
    monitor::Monitor,
};
use log::{debug, error, info};
use std::rc::Rc;
use transaction_dispatcher::config::ConfigAsLib as TxDispatcherConfig;

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
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .value_name("ENV")
                .help("Environment name (e.g., local, alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>("env").cloned();

    info!(
        "Loading configuration for environment: {}",
        env_name.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(env_name.clone()).expect("Failed to load config");

    let bitcoin_network = CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?;

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: TxDispatcherConfig =
        TxDispatcherConfig::load(env_name).expect("Failed to load transaction dispatcher config");

    let contract_addresses = config.get_contract_addresses();

    let block_broker = BrokerClient::new(
        config.block_broker.host,
        config.block_broker.port,
        config.broker_client_id,
    );
    let log_broker = BrokerClient::new(
        config.log_broker.host,
        config.log_broker.port,
        config.broker_client_id,
    );
    let user_broker = BrokerClient::new(
        config.user_broker.host,
        config.user_broker.port,
        config.broker_client_id,
    );
    let bitvmx_broker = Rc::new(BitVmxBrokerClient::new(
        config.bitvmx_broker.host,
        config.bitvmx_broker.port,
        BITVMX_L2_BROKER_CLIENT_ID,
    ));

    let monitor = Monitor::new(
        log_broker,
        block_broker,
        user_broker,
        bitvmx_broker.clone(),
        contract_addresses,
    );

    let shutdown_flag = ShutdownFlag::init();

    let rt_sync = RuntimeSync::new().context("Failed to create runtime sync")?;

    let contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib_sync(
        rt_sync.clone(),
        tx_dispatcher_config,
    )?;

    let store_path = &format!("{}/coordinator", config.storage_path);
    debug!("Creating coordinator store at: {}", store_path);
    let store = CoordinatorStore::new(store_path).context("Failed to create context store")?;

    let mut coordinator = Coordinator::new(
        rt_sync,
        monitor,
        contracts_gateway,
        bitvmx_broker,
        store,
        shutdown_flag.clone(),
        bitcoin_network,
    );
    coordinator.run().inspect_err(|e| {
        error!("Unrecoverable error running coordinator: {:?}", e);
        // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Shutting down!");

    Ok(())
}
