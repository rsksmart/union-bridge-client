use crate::TestWorld;
use crate::constants::FIXTURES_PATH;
use alloy_primitives::{Address, FixedBytes};
use alloy_provider::ProviderBuilder;
use anyhow::Context;
use bitcoin::Transaction;
use cucumber::gherkin::Step;
use cucumber::{then, when};
use qa_tools_common::common::extract_params;
use serde_json::json;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use union_contracts::bindings::peg_manager::PegManager;

#[when(expr = "bitvmx finds a pegin request")]
async fn bitvmx_finds_a_pegin_request(world: &mut TestWorld, step: &Step) {
    let params = extract_params(step);
    let file_name = params.get("file").unwrap().to_string();
    let btc_tx_file = format!("{}{}.json", FIXTURES_PATH, file_name);
    let json_str = std::fs::read_to_string(&btc_tx_file)
        .with_context(|| format!("Failed to read file: {}", btc_tx_file))
        .unwrap();
    let tx: Transaction = serde_json::from_str(&json_str)
        .context("Failed to parse BTC transaction JSON")
        .unwrap();
    let block_hash = params.get("block_hash").unwrap().to_string();
    let merkle_branch_path = params.get("merkle_branch_path").unwrap().to_string();
    let merkle_branch_hashes = vec![params.get("merkle_branch_hashes").unwrap().to_string()];
    world.pegin_request_tx_id = tx.compute_txid().to_string();
    world
        .bitvmx_mock
        .as_mut()
        .unwrap()
        .trigger_pegin_found(tx, block_hash, merkle_branch_path, merkle_branch_hashes)
        .unwrap();
    sleep(Duration::from_secs(3)).await;
}

#[when(expr = "bitvmx accepts a pegin request")]
async fn bitvmx_accepts_pegin_request(world: &mut TestWorld, step: &Step) {
    let params = extract_params(step);
    let file_name = params.get("file").unwrap().to_string();
    let btc_tx_file = format!("{}{}.json", FIXTURES_PATH, file_name);
    let json_template = std::fs::read_to_string(&btc_tx_file)
        .with_context(|| format!("Failed to read accept pegin template file: {}", btc_tx_file))
        .unwrap();
    let json_str = json_template.replace(
        "<REPLACE_WITH_peginRequestTxHash_FROM_PR_RESPONSE>",
        &world.pegin_request_tx_id,
    );
    let accept_tx: Transaction = serde_json::from_str(&json_str)
        .context("Failed to parse accept pegin BTC transaction JSON")
        .unwrap();
    let block_hash = params.get("block_hash").unwrap().to_string();
    let merkle_branch_path = params.get("merkle_branch_path").unwrap().to_string();
    let merkle_branch_hashes = vec![params.get("merkle_branch_hashes").unwrap().to_string()];
    world
        .bitvmx_mock
        .as_mut()
        .unwrap()
        .accept_pegin(
            accept_tx,
            block_hash,
            merkle_branch_path,
            merkle_branch_hashes,
        )
        .unwrap();
    sleep(Duration::from_secs(3)).await;
}
#[then(expr = "the pegin request should be registered in the contract")]
async fn pegin_request_should_be_registered(world: &mut TestWorld) {
    let provider = ProviderBuilder::new().connect_http(world.anvil_url.parse().unwrap());
    let peg_manager_address =
        Address::from_str(world.peg_manager_address.as_str()).expect("Invalid PegManager address");
    let peg_manager = PegManager::new(peg_manager_address, provider);
    let btc_tx_hash: FixedBytes<32> =
        FixedBytes::from_str(&world.pegin_request_tx_id).expect("Invalid BTC transaction hash");
    let mut last_peg_status = 0;
    let n_attempts = 5;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin registration (attempt {}/{})...",
            attempt, n_attempts
        );
        let stream_position = peg_manager
            .getStreamPosition(btc_tx_hash)
            .call()
            .await
            .expect("contract call failed");
        let peg_status = stream_position.pegStatus;
        println!("Stream position: {:?}", stream_position);
        println!("Stream status: {:?}", peg_status);
        last_peg_status = peg_status;
        sleep(Duration::from_secs(2)).await;
        if peg_status == 1 {
            break;
        }
    }
    assert_eq!(last_peg_status, 1, "Pegin not registered after 5 attempts.",);
}

#[when(expr = "enough confirmations are received")]
async fn enough_confirmations_received(world: &mut TestWorld) {
    let client = reqwest::Client::new();
    let blocks_to_mine = 5;
    println!("Mining {} blocks ...", blocks_to_mine);
    let anvil_url: String = world.anvil_url.clone();
    for i in 1..=blocks_to_mine {
        let request_payload = json!({
            "jsonrpc": "2.0",
            "method": "evm_mine",
            "params": [],
            "id": i
        });
        let response = client
            .post(&anvil_url)
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to send mining request {} to Anvil at {}",
                    i, anvil_url
                )
            })
            .unwrap();
        if !response.status().is_success() {
            println!("Failed to mine block {}: HTTP {}", i, response.status());
        }
        sleep(Duration::from_millis(100)).await;
    }
    println!("Successfully mined {} blocks.", blocks_to_mine);
    sleep(Duration::from_secs(6)).await;
}

#[then(expr = "the pegin accept should be registered in the contract")]
async fn pegin_request_should_be_accepted(world: &mut TestWorld) {
    let provider = ProviderBuilder::new().connect_http(world.anvil_url.parse().unwrap());
    let peg_manager_address =
        Address::from_str(world.peg_manager_address.as_str()).expect("Invalid PegManager address");
    let peg_manager = PegManager::new(peg_manager_address, provider);
    let btc_tx_hash: FixedBytes<32> =
        FixedBytes::from_str(&world.pegin_request_tx_id).expect("Invalid BTC transaction hash");
    let mut last_peg_status = 0;
    let n_attempts = 5;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin acceptance (attempt {}/{})...",
            attempt, n_attempts
        );
        let stream_position = peg_manager
            .getStreamPosition(btc_tx_hash)
            .call()
            .await
            .expect("contract call failed");
        let peg_status = stream_position.pegStatus;

        println!("Stream position: {:?}", stream_position);
        println!("Stream status: {:?}", peg_status);
        last_peg_status = peg_status;
        sleep(Duration::from_secs(2)).await;
        if peg_status == 2 {
            break;
        }
    }
    assert_eq!(last_peg_status, 2, "Pegin not accepted after 5 attempts.",);
}

#[then(expr = "the pegin request should be registered in the coordinator")]
async fn pegin_request_should_be_registered_in_the_coordinator(world: &mut TestWorld) {
    let n_attempts = 5;
    let mut last_pegin_requested_flow_id = None;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin request in coordinator (attempt {}/{})...",
            attempt, n_attempts
        );
        last_pegin_requested_flow_id = world
            .bitvmx_mock
            .as_ref()
            .unwrap()
            .get_last_pegin_requested_flow_id();

        if last_pegin_requested_flow_id.is_some() {
            break;
        }
        sleep(Duration::from_secs(2)).await;
    }
    assert!(
        last_pegin_requested_flow_id.is_some(),
        "No pegin request flow ID found in the coordinator mock after {} attempts.",
        n_attempts
    );
}

#[then(expr = "the pegin accept should be registered in the coordinator")]
async fn pegin_process_should_be_completed(world: &mut TestWorld) {
    let n_attempts = 5;
    let mut last_pegin_accepted_flow_id = None;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin accept in coordinator (attempt {}/{})...",
            attempt, n_attempts
        );
        last_pegin_accepted_flow_id = world
            .bitvmx_mock
            .as_ref()
            .unwrap()
            .get_last_pegin_accepted_flow_id();

        if last_pegin_accepted_flow_id.is_some() {
            break;
        }
        sleep(Duration::from_secs(2)).await;
    }
    assert!(
        last_pegin_accepted_flow_id.is_some(),
        "No pegin accepted flow ID found in the coordinator mock after {} attempts.",
        n_attempts
    );
}

#[then(expr = "the pegin request should not be registered in the contract")]
async fn pegin_request_should_not_be_registered(world: &mut TestWorld) {
    let provider = ProviderBuilder::new().connect_http(world.anvil_url.parse().unwrap());
    let peg_manager_address =
        Address::from_str(world.peg_manager_address.as_str()).expect("Invalid PegManager address");
    let peg_manager = PegManager::new(peg_manager_address, provider);
    let btc_tx_hash: FixedBytes<32> =
        FixedBytes::from_str(&world.pegin_request_tx_id).expect("Invalid BTC transaction hash");
    let n_attempts = 5;
    let mut last_peg_status = 0;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin registration (attempt {}/{})...",
            attempt, n_attempts
        );
        let stream_position = peg_manager
            .getStreamPosition(btc_tx_hash)
            .call()
            .await
            .expect("contract call failed");
        let peg_status = stream_position.pegStatus;
        println!("Stream position: {:?}", stream_position);
        println!("Stream status: {:?}", peg_status);
        sleep(Duration::from_secs(2)).await;
        if peg_status != 1 {
            last_peg_status = peg_status;
            break;
        }
    }
    assert_eq!(
        last_peg_status, 0,
        "Pegin registered in the contract, but it should not be."
    );
}

#[then(expr = "the pegin request should not be registered in the coordinator")]
async fn pegin_request_should_not_be_registered_in_the_coordinator(world: &mut TestWorld) {
    let n_attempts = 5;
    let mut last_pegin_requested_flow_id = None;
    for attempt in 1..=n_attempts {
        println!(
            "Checking pegin request in coordinator (attempt {}/{})...",
            attempt, n_attempts
        );
        let pegin_requested_flow_id = world
            .bitvmx_mock
            .as_ref()
            .unwrap()
            .get_last_pegin_requested_flow_id();

        if last_pegin_requested_flow_id.is_some() {
            last_pegin_requested_flow_id = pegin_requested_flow_id;
            break;
        }
        sleep(Duration::from_secs(2)).await;
    }
    assert!(
        last_pegin_requested_flow_id.is_none(),
        "Pegin registered in the coordinator, but it should not be."
    );
}
