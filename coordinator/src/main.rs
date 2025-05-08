use anyhow::{Result, anyhow, bail};
use clap::{Arg, Command};
use common::config::CommonConfig;
use common::msg_broker::broker::BrokerClient;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use log::{debug, error, info, trace};
use std::thread;

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

    let shutdown_flag = ShutdownFlag::init();
    start_monitoring(&block_broker, &log_broker, shutdown_flag)?;

    info!("Shutting down monitoring");
    Ok(())
}

fn start_monitoring(
    block_broker: &BrokerClient,
    log_broker: &BrokerClient,
    shutdown_flag: ShutdownFlag,
) -> Result<()> {
    request_block_monitoring(block_broker)?;
    request_log_monitoring(log_broker)?;

    loop {
        if shutdown_flag.is_on() {
            info!("Shutdown requested, stopping block notifier");
            break;
        }

        try_new_log(log_broker)?;
        try_new_block(block_broker)?;

        thread::sleep(std::time::Duration::from_secs(5)); // TODO(iago) config
    }

    cancel_log_monitoring(log_broker)?;
    cancel_block_monitoring(block_broker)?;

    Ok(())
}

fn try_new_block(block_broker: &BrokerClient) -> Result<()> {
    // TODO(iago) retries, etc.
    match block_broker.try_recv()? {
        Some(BrokerResponses::Block(b)) => {
            info!("Received new Block from Block Notifier {:?}", b)
        }
        Some(e) => {
            error!("Unexpected response type from Block Notifier {:?}", e);
        }
        None => trace!("No messages from Block Notifier"),
    }

    Ok(())
}

fn request_block_monitoring(block_broker: &BrokerClient) -> Result<()> {
    let result = block_broker.send(1, BrokerRequests::SubscribeBlocks)?; // TODO(iago) config
    if !result {
        bail!("Could not subscribe to blocks")
    }
    Ok(())
}

fn cancel_block_monitoring(block_broker: &BrokerClient) -> Result<()> {
    let result = block_broker.send(1, BrokerRequests::UnsubscribeBlocks)?; // TODO(iago) config
    if !result {
        bail!("Could not unsubscribe from blocks")
    }
    Ok(())
}

fn try_new_log(log_broker: &BrokerClient) -> Result<()> {
    // TODO(iago) retries, etc.
    match log_broker.try_recv()? {
        Some(BrokerResponses::Log(b)) => {
            info!("Received new Log from Log Notifier {:?}", b)
        }
        Some(e) => {
            error!("Unexpected response type from Log Notifier {:?}", e);
        }
        None => trace!("No messages from Log Notifier"),
    }

    Ok(())
}

fn request_log_monitoring(log_broker: &BrokerClient) -> Result<()> {
    let result = log_broker.send(1, BrokerRequests::SubscribeLogs("test_topic".to_string()))?; // TODO(iago) config
    if !result {
        bail!("Could not subscribe to logs")
    }
    Ok(())
}

fn cancel_log_monitoring(log_broker: &BrokerClient) -> Result<()> {
    let result = log_broker.send(1, BrokerRequests::UnsubscribeLogs("test_topic".to_string()))?; // TODO(iago) config
    if !result {
        bail!("Could not unsubscribe from logs")
    }
    Ok(())
}
