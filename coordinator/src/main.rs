use std::rc::Rc;

use anyhow::{Context, Result, ensure};
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
const CONFIG_CLI_FLAG: &str = "config";

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

fn require_pubkey_hash(pubkey_hash: &str, service: &str) -> Result<String> {
    ensure!(
        !pubkey_hash.is_empty() && pubkey_hash != "<to_patch_with_env>",
        "{service} broker must be configured with a broker pubkey_hash"
    );
    Ok(pubkey_hash.to_owned())
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
            Arg::new(CONFIG_CLI_FLAG)
                .short('e')
                .long(CONFIG_CLI_FLAG)
                .value_name("CONFIG")
                .help("Configuration profile name (e.g., local, docker, alphanet)"),
        )
        .get_matches();
    Logger::init(matches.get_one::<String>(LOGGER_CLI_FLAG)).expect("Failed to load logger");
    matches.get_one::<String>(CONFIG_CLI_FLAG).cloned()
}

fn main() -> Result<()> {
    let config_name = parse_cli_args();

    info!(
        "Loading configuration profile: {}. Env vars with prefix UB__ will override config values",
        config_name.clone().unwrap_or_else(|| "NONE".to_string())
    );

    let config: Config = Config::load(config_name.as_deref()).expect("Failed to load config");
    info!("Loaded runtime environment: {}", config.environment);

    let bitcoin_network = CommonConfig::parse_bitcoin_network(&config.bitcoin_network)?;

    let tx_dispatcher_config: TxDispatcherConfig = TxDispatcherConfig::load(config_name.clone())
        .expect("Failed to load transaction dispatcher config");

    let contract_addresses = config.get_contract_addresses();
    let broker_key_path = &config.coordinator.broker.key_path;
    let coordinator_pubkey_hash = Cert::from_key_file(broker_key_path)
        .context("Failed to load coordinator broker key")?
        .get_pubk_hash()
        .context("Failed to derive coordinator broker pubkey_hash")?;

    info!("Coordinator broker client identity pubkey_hash={coordinator_pubkey_hash}");

    let broker_client_id = u8::try_from(config.coordinator.broker.client_id)
        .context("broker.client_id must fit in u8")?;

    let block_pubkey_hash = require_pubkey_hash(&config.coordinator.blocks.pubkey_hash, "blocks")?;
    debug!("Coordinator block broker target pubkey_hash={block_pubkey_hash}");
    let block_broker = create_broker(
        config.coordinator.blocks.host,
        config.coordinator.blocks.port,
        block_pubkey_hash,
        broker_client_id,
        broker_key_path,
        "block",
    )?;

    let log_pubkey_hash = require_pubkey_hash(&config.coordinator.logs.pubkey_hash, "logs")?;
    debug!("Coordinator log broker target pubkey_hash={log_pubkey_hash}");
    let log_broker = create_broker(
        config.coordinator.logs.host,
        config.coordinator.logs.port,
        log_pubkey_hash,
        broker_client_id,
        broker_key_path,
        "log",
    )?;

    let user_pubkey_hash = require_pubkey_hash(&config.coordinator.user.pubkey_hash, "user")?;
    debug!("Coordinator user broker target pubkey_hash={user_pubkey_hash}");
    let user_broker = create_broker(
        config.coordinator.user.host,
        config.coordinator.user.port,
        user_pubkey_hash,
        broker_client_id,
        broker_key_path,
        "user",
    )?;

    let bitvmx_pubkey_hash = require_pubkey_hash(&config.coordinator.bitvmx.pubkey_hash, "bitvmx")?;
    debug!("Coordinator BitVMX broker target pubkey_hash={bitvmx_pubkey_hash}");
    let bitvmx_broker = Rc::new(
        BitVmxBrokerClient::new(
            config.coordinator.bitvmx.host.clone(),
            config.coordinator.bitvmx.port,
            bitvmx_pubkey_hash,
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
        &config.environment,
        &config.bridge,
    );
    coordinator.run().inspect_err(|e| {
        error!("Unrecoverable error running coordinator: {e:?}"); // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Shutting down!");
    Ok(())
}
