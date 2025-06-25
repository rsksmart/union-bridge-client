mod step_definitions;
mod steps;
mod constants;
mod setup;
mod teardown;

use cucumber::{World, writer::JUnit};
use std::env;
use std::fs::File;
use std::process::Child;
use std::time::Duration;
use crate::setup::{deploy_contracts, setup_anvil, setup_transaction_dispatcher, transfer_funds};
use crate::teardown::{shutdown_anvil, shutdown_transaction_dispatcher};

const DEPLOY_LOCAL_CONTRACTS_PATH_DEFAULT: &str = "../../bitvmx-union-bridge-contracts/shell/script/deploy/deploy-local.sh";
const ANVIL_DOMAIN_DEFAULT: &str = "http://localhost";
const ANVIL_PORT_DEFAULT: u16 = 8545;
const ANVIL_ADRESS_DEFAULT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const KEY_STORE_ADDRESS_DEFAULT: &str = "0x5bdd03ceaf59cad075cb29c67696581d857b9031";
const KEY_STORE_PASSWORD_DEFAULT: &str = "p09ol.";
const FUNDS_AMOUNT_WEI: &str = "1000000000000000000"; // 1 ETH
const ANVIL_TIMEOUT: Duration = Duration::from_secs(5);
const TX_DISPATCHER_MANIFEST_RELATIVE_PATH: &str = "../transaction-dispatcher/Cargo.toml";
const TX_DISPATCHER_URL_DEFAULT: &str = "http://localhost:3000";
const TX_DISPATCHER_CONFIG_PATH_DEFAULT: &str = "../config/qa-local";
const TX_DISPATCHER_TIMEOUT: Duration = Duration::from_secs(300);

lazy_static::lazy_static! {
    pub static ref DEPLOY_LOCAL_CONTRACTS_PATH: String = env::var("DEPLOY_LOCAL_CONTRACTS_PATH")
        .unwrap_or_else(|_| DEPLOY_LOCAL_CONTRACTS_PATH_DEFAULT.to_string());
    pub static ref ANVIL_DOMAIN: String = env::var("ANVIL_DOMAIN")
        .unwrap_or_else(|_| ANVIL_DOMAIN_DEFAULT.to_string());
    pub static ref ANVIL_PORT: String = env::var("ANVIL_PORT")
        .unwrap_or_else(|_| ANVIL_PORT_DEFAULT.to_string());
    pub static ref ANVIL_URL: String = format!("{}:{}", ANVIL_DOMAIN.as_str(), ANVIL_PORT.as_str());
    pub static ref ANVIL_ADDRESS: String = env::var("ANVIL_ADDRESS")
        .unwrap_or_else(|_| ANVIL_ADRESS_DEFAULT.to_string());
    pub static ref KEY_STORE_ADDRESS: String = env::var("KEY_STORE_ADDRESS")
        .unwrap_or_else(|_| KEY_STORE_ADDRESS_DEFAULT.to_string());
    pub static ref KEY_STORE_PASSWORD: String = env::var("KEY_STORE_PASSWORD")
        .unwrap_or_else(|_| KEY_STORE_PASSWORD_DEFAULT.to_string());
    pub static ref TX_DISPATCHER_URL: String = env::var("TX_DISPATCHER_URL")
        .unwrap_or_else(|_| TX_DISPATCHER_URL_DEFAULT.to_string());
        pub static ref TX_DISPATCHER_CONFIG_PATH: String = env::var("TX_DISPATCHER_CONFIG_PATH")
        .unwrap_or_else(|_| TX_DISPATCHER_CONFIG_PATH_DEFAULT.to_string());
}

#[derive(Debug, Default, World)]
pub struct TestWorld {
    pub child_anvil: Option<Child>,
    pub child_tx_dispatcher: Option<Child>,
    pub response: Option<String>,
    pub status_code: Option<u16>,
    pub response_2: Option<String>,
    pub status_code_2: Option<u16>,
    pub register_pegin_block_hash: Option<String>,
    pub register_pegin_btc_txid: Option<String>,
    pub register_pegin_merkle_hash: Option<String>,
}

#[tokio::main]
async fn main() {
    let junit_report = env::var("JUNIT_REPORT");
    if junit_report.is_ok() {
        let report_file = File::create(junit_report.unwrap()).unwrap();
        println!("Running tests with JUnit report at: {:?}", report_file);
        TestWorld::cucumber()
            .with_writer(JUnit::new(report_file, 0))
            .max_concurrent_scenarios(Some(1)) // Run in sequence to avoid conflicts between scenarios
            .before(|_, _, _, world: &mut TestWorld| {
                Box::pin(async move {
                    tx_dispatcher_setup(world).await;
                })
            })
            .after(|_, _, _, _, world_opt: Option<&mut TestWorld>| {
                Box::pin(async move {
                    if let Some(world) = world_opt {
                        tx_dispatcher_teardown(world).await;
                    }
                })
            })
            .run("transaction-dispatcher/features")
            .await;
    } else {
        TestWorld::cucumber()
            .init_tracing()
            .max_concurrent_scenarios(Some(1)) // Run in sequence to avoid conflicts between scenarios
            .before(|_, _, _, world: &mut TestWorld| {
                Box::pin(async move {
                    tx_dispatcher_setup(world).await;
                })
            })
            .after(|_, _, _, _, world_opt: Option<&mut TestWorld>| {
                Box::pin(async move {
                    if let Some(world) = world_opt {
                        tx_dispatcher_teardown(world).await;
                    }
                })
            })
            .run_and_exit("transaction-dispatcher/features")
            .await;
    }
}

async fn tx_dispatcher_setup(world: &mut TestWorld) {
    println!("*** SETUP *** Setting up transaction dispatcher environment...");
    let anvil_port: u16 = ANVIL_PORT.parse().unwrap();
    let child_anvil: Child = setup_anvil(&ANVIL_URL, anvil_port, ANVIL_TIMEOUT).await;
    deploy_contracts(DEPLOY_LOCAL_CONTRACTS_PATH.as_str());
    transfer_funds(
        &ANVIL_URL,
        &ANVIL_ADDRESS,
        &KEY_STORE_ADDRESS,
        FUNDS_AMOUNT_WEI,
    );
    let child_tx_dispatcher: Child = setup_transaction_dispatcher(
        &KEY_STORE_PASSWORD,
        &TX_DISPATCHER_URL,
        TX_DISPATCHER_MANIFEST_RELATIVE_PATH,
        &TX_DISPATCHER_CONFIG_PATH,
        TX_DISPATCHER_TIMEOUT,
    )
    .await;
    world.child_anvil = Some(child_anvil);
    world.child_tx_dispatcher = Some(child_tx_dispatcher);
}

async fn tx_dispatcher_teardown(world: &mut TestWorld) {
    println!("*** TEARDOWN *** Shutting down transaction dispatcher environment...");
    shutdown_anvil(world.child_anvil.take());
    shutdown_transaction_dispatcher(world.child_tx_dispatcher.take());
}
