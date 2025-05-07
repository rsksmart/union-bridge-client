use anyhow::{Result, anyhow, bail};
use clap::{Arg, Command};
use common::config::CommonConfig;
use common::msg_broker::broker::BrokerClient;
use common::msg_broker::types::{BrokerRequests, BrokerResponses};
use common::shutdown_flag::ShutdownFlag;
use log::{debug, info, trace};
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

    let shutdown_flag = ShutdownFlag::init();
    let msg = monitor_blocks(&block_broker, shutdown_flag)?;
    info!("Received message from channel: {:?}", msg);
    Ok(())
}

pub fn monitor_blocks(block_broker: &BrokerClient, shutdown_flag: ShutdownFlag) -> Result<()> {
    request_block_monitoring(block_broker)?;

    // TODO(iago) retries, etc.

    let mut num_blocks = 0;
    loop {
        if shutdown_flag.is_on() {
            return Err(anyhow!("Shutdown requested")); // TODO(iago) type
        }

        // TODO(iago) tmp code
        if num_blocks == 3 {
            debug!("Done, {num_blocks} blocks received");
            break;
        }

        match block_broker.try_recv()? {
            Some(BrokerResponses::Block(b)) => {
                info!("Received new Block from Block Notifier {:?}", b);
                num_blocks += 1;
                // try to receive new ASAP
                continue;
            }
            Some(_) => {
                bail!("Unknown response type from Block Notifier");
            }
            None => {
                trace!("No messages from Block Notifier");
            }
        }

        thread::sleep(std::time::Duration::from_secs(5)); // TODO(iago) config
    }

    cancel_block_monitoring(block_broker)?;

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
        bail!("Could not subscribe to blocks")
    }
    Ok(())
}
