use qa_tools_common::common::{execute_command, execute_script_with_basedir, spawn_command};
use reqwest::Client;
use std::process::Child;
use std::time::Duration;
use tokio::time::{sleep, timeout};

pub async fn setup_anvil(anvil_url: &str, anvil_port: u16, anvil_timeout: Duration) -> Child {
    println!(" *** SETUP *** Setting up anvil at: {}", anvil_url);
    let command = format!("anvil --port {}", anvil_port);
    let anvil_child = spawn_command(&command);
    wait_for_anvil(anvil_url, anvil_timeout).await;
    println!(
        " *** SETUP *** Anvil is up and running at: {}, with PID: {}",
        anvil_url,
        anvil_child.id()
    );
    anvil_child
}

pub fn deploy_contracts(contracts_base_dir: &str, deploy_local_path: &str) {
    println!(
        " *** SETUP *** Deploying contracts in base dir: {} and relative path: {}",
        contracts_base_dir, deploy_local_path
    );
    execute_script_with_basedir(contracts_base_dir, deploy_local_path);
}

pub fn packet_creation_flow(contracts_base_dir: &str, packet_creation_flow_relative_path: &str) {
    println!(
        " *** SETUP *** Executing packet creation flow in base dir: {} and relative path: {}",
        contracts_base_dir, packet_creation_flow_relative_path
    );
    execute_script_with_basedir(contracts_base_dir, packet_creation_flow_relative_path);
}

pub fn transfer_funds(anvil_url: &str, from: &str, to: &str, amount: &str) {
    println!(
        "*** SETUP *** Transferring funds from {} to {} with amount: {}",
        from, to, amount
    );
    let command = format!(
        "cast send --rpc-url {} --from {} {} --value {} --unlocked",
        anvil_url, from, to, amount
    );
    execute_command(&command);
    // TODO: in tesnet environment, we should wait for the transaction to be mined
}

pub async fn setup_transaction_dispatcher(
    key_store_passwd: &str,
    tx_dispatcher_url: &str,
    tx_dispatcher_manifest_path: &str,
    tx_dispatcher_config_path: &str,
    tx_dispatcher_timeout: Duration,
) -> Child {
    let command = format!(
        "KEY_STORE_PASSWORD={} cargo run --manifest-path {} --bin transaction-dispatcher -- --config-path {}",
        key_store_passwd, tx_dispatcher_manifest_path, tx_dispatcher_config_path
    );
    println!(
        "*** SETUP *** Setting up transaction dispatcher at url: {} with command: {}",
        tx_dispatcher_url, command
    );
    let child = spawn_command(&command);
    wait_for_transaction_dispatcher(tx_dispatcher_url, tx_dispatcher_timeout).await;
    println!(
        "*** SETUP *** Transaction dispatcher is up and running at: {}, with PID: {}",
        tx_dispatcher_url,
        child.id()
    );
    child
}

async fn wait_for_anvil(anvil_url: &str, anvil_timeout: Duration) {
    let client = Client::new();
    let waiter = async {
        loop {
            if client.get(anvil_url).send().await.is_ok() {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    };
    timeout(anvil_timeout, waiter)
        .await
        .unwrap_or_else(|_| panic!("Anvil did not start within {:?} seconds", anvil_timeout));
}

async fn wait_for_transaction_dispatcher(tx_dispatcher_url: &str, tx_dispatcher_timeout: Duration) {
    let client = Client::new();
    let url = format!("{}/pegin-address", tx_dispatcher_url);
    let waiter = async {
        loop {
            if let Ok(resp) = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json("{}")
                .send()
                .await
            {
                if resp.status().is_client_error() {
                    return;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    };
    timeout(tx_dispatcher_timeout, waiter)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "Transaction dispatcher did not start within {:?} seconds",
                tx_dispatcher_timeout
            )
        });
}
