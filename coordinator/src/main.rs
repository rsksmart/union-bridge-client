use std::rc::Rc;

use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::config::CommonConfig;
use common::msg_broker::broker::{
    BITVMX_L2_BROKER_CLIENT_ID, BitVmxBrokerClient, BrokerClient, Cert,
};
use common::runtime_sync::RuntimeSync;
use common::shutdown_flag::ShutdownFlag;
use coordinator::config::{Config, Logger};
use coordinator::coordinator::Coordinator;
use coordinator::monitor::Monitor;
use coordinator::store::CoordinatorStore;
use log::{debug, error, info};
use transaction_dispatcher::config::Config as TxDispatcherConfig;

const LOGGER_CLI_FLAG: &str = "logger-path";
const ENV_CLI_FLAG: &str = "env";

fn create_broker(
    host: String,
    port: u16,
    pubk_hash: String,
    client_id: u8,
    key_path: &str,
    name: &str,
) -> Result<BrokerClient> {
    BrokerClient::new(host, port, pubk_hash, client_id, key_path)
        .context(format!("Failed to create {name} broker client"))
}

fn parse_cli_args() -> Option<String> {
    let matches = Command::new("Union Bridge Coordinator")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(ENV_CLI_FLAG)
                .short('e')
                .long(ENV_CLI_FLAG)
                .value_name("ENV")
                .help("Environment name (e.g., local, alphanet, stage)"),
        )
        .get_matches();
    Logger::init(matches.get_one::<String>(LOGGER_CLI_FLAG)).expect("Failed to load logger");
    matches.get_one::<String>(ENV_CLI_FLAG).cloned()
}

fn main() -> Result<()> {
    let env_name = parse_cli_args();

    info!(
        "Loading configuration for environment: {}. Env vars with prefix UB__ will override config values",
        env_name.clone().unwrap_or_else(|| "NONE".to_string())
    );

    let config: Config = Config::load(env_name.as_deref()).expect("Failed to load config");

    let bitcoin_network = CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?;

    let tx_dispatcher_config: TxDispatcherConfig = TxDispatcherConfig::load(env_name.clone())
        .expect("Failed to load transaction dispatcher config");

    let contract_addresses = config.get_contract_addresses();
    let broker_key_path = &config.key_store.broker_key_path;

    let broker_server_pubk_hash = Cert::from_key_file(broker_key_path)
        .context("Failed to load broker key for pubkey_hash")?
        .get_pubk_hash()
        .context("Failed to compute broker pubkey_hash")?;

    let broker_client_id = u8::try_from(config.coordinator.broker.client_id)
        .context("broker.client_id must fit in u8")?;

    let hash = &broker_server_pubk_hash;
    let block_broker = create_broker(
        config.coordinator.blocks.host,
        config.coordinator.blocks.port,
        hash.clone(),
        broker_client_id,
        broker_key_path,
        "block",
    )?;
    let log_broker = create_broker(
        config.coordinator.logs.host,
        config.coordinator.logs.port,
        hash.clone(),
        broker_client_id,
        broker_key_path,
        "log",
    )?;
    let user_broker = create_broker(
        config.coordinator.user.host,
        config.coordinator.user.port,
        hash.clone(),
        broker_client_id,
        broker_key_path,
        "user",
    )?;

    let bitvmx_broker = Rc::new(
        BitVmxBrokerClient::new(
            config.coordinator.bitvmx.host.clone(),
            config.coordinator.bitvmx.port,
            config.coordinator.bitvmx.pubkey_hash.clone(),
            BITVMX_L2_BROKER_CLIENT_ID,
            broker_key_path,
        )
        .context("Failed to create BitVMX broker client")?,
    );

    let monitor = Monitor::new(
        log_broker,
        block_broker,
        user_broker,
        bitvmx_broker.clone(),
        contract_addresses,
    );

    let shutdown_flag = ShutdownFlag::init();

    let rt_sync = RuntimeSync::new().context("Failed to create runtime sync")?;

    let contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib_sync_with_role(
        &rt_sync,
        tx_dispatcher_config,
        transaction_dispatcher::GatewayRole::Member, // Coordinator uses member role
    )?;

    let store_path = &format!("{}/coordinator", config.coordinator.storage_path);
    debug!("Creating coordinator store at: {store_path}");
    let store = CoordinatorStore::new(store_path).context("Failed to create context store")?;

    let mut coordinator = Coordinator::new(
        &rt_sync,
        monitor,
        contracts_gateway,
        &bitvmx_broker,
        config.coordinator.advance_funds.clone(),
        store,
        shutdown_flag.clone(),
        bitcoin_network,
        env_name.as_deref(),
        &config.bridge,
    );
    coordinator.run().inspect_err(|e| {
        error!("Unrecoverable error running coordinator: {e:?}"); // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Shutting down!");
    Ok(())
}
