mod constants;
mod setup;
mod step_definitions;
mod teardown;

use crate::setup::{deploy_contracts, forge_clean, packet_creation_flow, setup_anvil, setup_block_indexer, setup_coordinator, setup_log_indexer, setup_transaction_dispatcher, setup_user_api, transfer_funds};
use crate::teardown::{
    shutdown_anvil, shutdown_bitvmx_mock, shutdown_block_indexer, shutdown_coordinator,
    shutdown_log_indexer, shutdown_transaction_dispatcher, shutdown_user_api,
};
use cucumber::{World, writer::JUnit};
use qa_tools_bitvmx_mock::AutomatedBitVmxMock;
use std::env;
use std::fs::File;
use std::process::Child;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

const CONTRACTS_BASEDIR_DEFAULT: &str = "../../bitvmx-union-bridge-contracts";
const DEPLOY_LOCAL_CONTRACTS_RELATIVE_PATH_DEFAULT: &str = "shell/script/deploy/deploy-local.sh";
const PACKET_CREATION_FLOW_RELATIVE_PATH_DEFAULT: &str =
    "shell/script/integration-test/packet-creation-flow.sh";
const ANVIL_DOMAIN_DEFAULT: &str = "http://localhost";
const ANVIL_PORT_DEFAULT: u16 = 8545;
const ANVIL_ADDRESS_DEFAULT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const PEG_MANAGER_ADDRESS: &str = "0x0165878A594ca255338adfa4d48449f69242Eb8F";
const FUNDS_AMOUNT_WEI: &str = "1000000000000000000"; // 1 ETH
const ANVIL_TIMEOUT: Duration = Duration::from_secs(5);
const CONFIG_PATH_DEFAULT: &str = "../config/qa";

const TX_DISPATCHER_MANIFEST_RELATIVE_PATH: &str = "../transaction-dispatcher/Cargo.toml";
const TX_DISPATCHER_URL_DEFAULT: &str = "http://localhost:3000";
const TIMEOUT: Duration = Duration::from_secs(300);
const BITVMX_PORT_DEFAULT: u16 = 9094;
const BLOCK_INDEXER_MANIFEST_RELATIVE_PATH: &str = "../block-indexer/Cargo.toml";
const LOG_INDEXER_MANIFEST_RELATIVE_PATH: &str = "../log-indexer/Cargo.toml";
const COORDINATOR_MANIFEST_RELATIVE_PATH: &str = "../coordinator/Cargo.toml";
const USER_API_MANIFEST_RELATIVE_PATH: &str = "../user-api/Cargo.toml";

lazy_static::lazy_static! {
    pub static ref DEPLOY_LOCAL_CONTRACTS_RELATIVE_PATH: String = env::var("DEPLOY_LOCAL_CONTRACTS_RELATIVE_PATH")
        .unwrap_or_else(|_| DEPLOY_LOCAL_CONTRACTS_RELATIVE_PATH_DEFAULT.to_string());
    pub static ref PACKET_CREATION_FLOW_RELATIVE_PATH: String = env::var("PACKET_CREATION_FLOW_RELATIVE_PATH")
        .unwrap_or_else(|_| PACKET_CREATION_FLOW_RELATIVE_PATH_DEFAULT.to_string());
    pub static ref CONTRACTS_BASEDIR: String = env::var("CONTRACTS_BASEDIR")
        .unwrap_or_else(|_| CONTRACTS_BASEDIR_DEFAULT.to_string());
    pub static ref ANVIL_DOMAIN: String = env::var("ANVIL_DOMAIN")
        .unwrap_or_else(|_| ANVIL_DOMAIN_DEFAULT.to_string());
    pub static ref ANVIL_PORT: String = env::var("ANVIL_PORT")
        .unwrap_or_else(|_| ANVIL_PORT_DEFAULT.to_string());
    pub static ref ANVIL_URL: String = format!("{}:{}", ANVIL_DOMAIN.as_str(), ANVIL_PORT.as_str());
    pub static ref ANVIL_ADDRESS: String = env::var("ANVIL_ADDRESS")
        .unwrap_or_else(|_| ANVIL_ADDRESS_DEFAULT.to_string());
    pub static ref KEY_STORE_ADDRESS: String = env::var("KEY_STORE_ADDRESS")
        .unwrap_or_else(|_| Err("KEY_STORE_ADDRESS environment variable is not set").unwrap());
    pub static ref KEY_STORE_PASSWORD: String = env::var("KEY_STORE_PASSWORD")
        .unwrap_or_else(|_| Err("KEY_STORE_PASSWORD environment variable is not set").unwrap());
    pub static ref TX_DISPATCHER_URL: String = env::var("TX_DISPATCHER_URL")
        .unwrap_or_else(|_| TX_DISPATCHER_URL_DEFAULT.to_string());
    pub static ref TX_DISPATCHER_CONFIG_PATH: String = env::var("TX_DISPATCHER_CONFIG_PATH")
        .unwrap_or_else(|_| CONFIG_PATH_DEFAULT.to_string());
    pub static ref BLOCK_INDEXER_CONFIG_PATH: String = env::var("BLOCK_INDEXER_CONFIG_PATH")
        .unwrap_or_else(|_| CONFIG_PATH_DEFAULT.to_string());
    pub static ref LOG_INDEXER_CONFIG_PATH: String = env::var("LOG_INDEXER_CONFIG_PATH")
        .unwrap_or_else(|_| CONFIG_PATH_DEFAULT.to_string());
    pub static ref COORDINATOR_CONFIG_PATH: String = env::var("COORDINATOR_CONFIG_PATH")
        .unwrap_or_else(|_| CONFIG_PATH_DEFAULT.to_string());
    pub static ref USER_API_CONFIG_PATH: String = env::var("USER_API_CONFIG_PATH")
        .unwrap_or_else(|_| CONFIG_PATH_DEFAULT.to_string());
    pub static ref BITVMX_PORT: String = env::var("BITVMX_MOCK_PORT")
        .unwrap_or_else(|_| BITVMX_PORT_DEFAULT.to_string());
}

#[derive(Debug, Default, World)]
pub struct TestWorld {
    pub child_anvil: Option<Child>,
    pub child_block_indexer: Option<Child>,
    pub child_log_indexer: Option<Child>,
    pub child_tx_dispatcher: Option<Child>,
    pub child_coordinator: Option<Child>,
    pub child_user_api: Option<Child>,
    pub bitvmx_mock: Option<Arc<AutomatedBitVmxMock>>,
    pub anvil_url: String,
    pub peg_manager_address: String,
    pub pegin_request_tx_id: String,
    pub pegin_accept_tx_id: String,
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
                    setup(world).await;
                })
            })
            .after(|_, _, _, _, world_opt: Option<&mut TestWorld>| {
                Box::pin(async move {
                    if let Some(world) = world_opt {
                        teardown(world).await;
                        let junit_report = env::var("JUNIT_REPORT");
                        let report_path = junit_report.unwrap();
                        let report = std::fs::read_to_string(report_path).unwrap();
                        println!("{}", report);
                    }
                })
            })
            .run("coordinator/features")
            .await;
    } else {
        TestWorld::cucumber()
            // .init_tracing()
            .max_concurrent_scenarios(Some(1)) // Run in sequence to avoid conflicts between scenarios
            .before(|_, _, _, world: &mut TestWorld| {
                Box::pin(async move {
                    setup(world).await;
                })
            })
            .after(|_, _, _, _, world_opt: Option<&mut TestWorld>| {
                Box::pin(async move {
                    if let Some(world) = world_opt {
                        teardown(world).await;
                    }
                })
            })
            .run_and_exit("coordinator/features")
            .await;
    }
}

async fn setup(world: &mut TestWorld) {
    println!("*** SETUP *** Setting up environment...");
    world.anvil_url = ANVIL_URL.clone();
    world.peg_manager_address = PEG_MANAGER_ADDRESS.to_string();

    let anvil_port: u16 = ANVIL_PORT.parse().unwrap();
    let child_anvil: Child = setup_anvil(&ANVIL_URL, anvil_port, ANVIL_TIMEOUT).await;
    world.child_anvil = Some(child_anvil);
    forge_clean(CONTRACTS_BASEDIR.as_str());
    deploy_contracts(
        CONTRACTS_BASEDIR.as_str(),
        DEPLOY_LOCAL_CONTRACTS_RELATIVE_PATH.as_str(),
    );
    packet_creation_flow(
        CONTRACTS_BASEDIR.as_str(),
        PACKET_CREATION_FLOW_RELATIVE_PATH.as_str(),
    );
    transfer_funds(
        &ANVIL_URL,
        &ANVIL_ADDRESS,
        &KEY_STORE_ADDRESS,
        FUNDS_AMOUNT_WEI,
    );
    let bitvmx_port: u16 = BITVMX_PORT.parse().unwrap();
    let mut bitvmx_mock = AutomatedBitVmxMock::new(bitvmx_port);
    bitvmx_mock
        .start()
        .await
        .expect("BitVMX mock server failed to start");
    world.bitvmx_mock = Some(bitvmx_mock);

    let child_block_indexer: Child = setup_block_indexer(
        BLOCK_INDEXER_MANIFEST_RELATIVE_PATH,
        &BLOCK_INDEXER_CONFIG_PATH,
        TIMEOUT,
    )
    .await;
    world.child_block_indexer = Some(child_block_indexer);
    let child_log_indexer: Child = setup_log_indexer(
        LOG_INDEXER_MANIFEST_RELATIVE_PATH,
        &LOG_INDEXER_CONFIG_PATH,
        TIMEOUT,
    )
    .await;
    world.child_log_indexer = Some(child_log_indexer);
    let child_tx_dispatcher: Child = setup_transaction_dispatcher(
        &KEY_STORE_PASSWORD,
        &TX_DISPATCHER_URL,
        TX_DISPATCHER_MANIFEST_RELATIVE_PATH,
        &TX_DISPATCHER_CONFIG_PATH,
        TIMEOUT,
    )
    .await;
    let child_user_api: Child = setup_user_api(
        USER_API_MANIFEST_RELATIVE_PATH,
        &USER_API_CONFIG_PATH,
        TIMEOUT,
    )
    .await;
    world.child_user_api = Some(child_user_api);
    world.child_tx_dispatcher = Some(child_tx_dispatcher);
    let child_coordinator: Child = setup_coordinator(
        COORDINATOR_MANIFEST_RELATIVE_PATH,
        &COORDINATOR_CONFIG_PATH,
        TIMEOUT,
    )
    .await;
    world.child_coordinator = Some(child_coordinator);
    sleep(Duration::from_secs(6)); // allow time for async pegin flow
}

async fn teardown(world: &mut TestWorld) {
    println!("*** TEARDOWN *** Shutting down environment...");
    shutdown_anvil(world.child_anvil.take());
    shutdown_coordinator(world.child_coordinator.take());

    shutdown_user_api(&mut world.child_user_api);
    shutdown_transaction_dispatcher(world.child_tx_dispatcher.take());
    shutdown_log_indexer(world.child_log_indexer.take());
    shutdown_block_indexer(world.child_block_indexer.take());

    shutdown_bitvmx_mock(world.bitvmx_mock.take()).await;
}
