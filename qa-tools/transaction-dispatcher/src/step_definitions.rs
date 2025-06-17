use cucumber::gherkin::Step;
use cucumber::{then, when};
use qa_tools_common::common::extract_params;
use serde_json::Value;
use crate::steps::{call_endpoint, extract_addresses};
use crate::TestWorld;

#[when(expr = "I POST to {string}")]
async fn post_endpoint(world: &mut TestWorld, endpoint: String, step: &Step) {
    let params = extract_params(step);
    let (status, response_text) = call_endpoint(&params, endpoint, world).await;
    world.status_code = Some(status.as_u16());
    world.response = Some(response_text);
}

#[when(expr = "I POST to {string} again")]
async fn post_endpoint_again(world: &mut TestWorld, endpoint: String, step: &Step) {
    let params = extract_params(step);
    let (status, response_text) = call_endpoint(&params, endpoint, world).await;
    world.status_code_2 = Some(status.as_u16());
    world.response_2 = Some(response_text);
}

#[then(expr = "the response code should be {string}")]
async fn response_code_should_be(world: &mut TestWorld, expected_code: String) {
    let code = expected_code.parse::<u16>().unwrap();
    let response_text = world.response.as_ref().expect("No response received");
    assert_eq!(
        world.status_code,
        Some(code),
        "Expected status code {} but got {}\nResponse: {}",
        code,
        world.status_code.unwrap(),
        response_text
    );
}

#[then(expr = "the response code of both responses should be {string}")]
async fn response_code_of_both_responses_should_be(world: &mut TestWorld, expected_code: String) {
    let code = expected_code.parse::<u16>().unwrap();
    assert_eq!(
        world.status_code,
        Some(code),
        "Expected status code {} but got {}",
        code,
        world.status_code.unwrap()
    );
    assert_eq!(
        world.status_code_2,
        Some(code),
        "Expected status code 2 {} but got {}",
        code,
        world.status_code_2.unwrap()
    );
}

#[then(expr = "the response should contain the bitcoin address {string}")]
async fn response_should_contain_bitcoin_address(world: &mut TestWorld, btc_address: String) {
    let response_text = world.response.as_ref().expect("No response received");
    let json: Value = serde_json::from_str(response_text).expect("response was not valid JSON");
    assert_eq!(
        json["address"].as_str(),
        Some(btc_address.as_str()),
        "Response does not contain the expected address"
    );
}

#[then(expr = "the response should contain a valid transaction hash")]
async fn response_should_contain_transaction_hash(world: &mut TestWorld) {
    let response_text = world.response.as_ref().expect("No response received");
    let json: Value = serde_json::from_str(response_text).expect("response was not valid JSON");

    let tx_hash = json["transaction_hash"]
        .as_str()
        .expect("response JSON has no `transaction_hash` field");

    assert!(
        tx_hash.starts_with("0x"),
        "Transaction hash should start with 0x"
    );
    let hex_part = &tx_hash[2..];
    assert!(
        hex_part.len() == 64,
        "Transaction hash should be 32 bytes (64 hex chars)"
    );
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "Transaction hash should only contain hex characters"
    );
}

#[then(expr = "the response should contain the error {string}")]
async fn response_should_contain_error(world: &mut TestWorld, expected_error: String) {
    let response_text = world.response.as_ref().expect("No response received");
    let json: Value = serde_json::from_str(response_text).expect("response was not valid JSON");
    let err_msg = json["error"]
        .as_str()
        .expect("response JSON has no string `error` field");
    assert!(
        err_msg.contains(&expected_error),
        "Expected error `{}` but got `{}`",
        expected_error,
        err_msg
    );
}

#[then("the addresses of both responses should be equal")]
async fn check_addresses_equal(world: &mut TestWorld) {
    let (address1, address2) = extract_addresses(world);
    assert_eq!(address1, address2, "Addresses are not equal");
}

#[then("the addresses of both responses should be different")]
async fn check_addresses_different(world: &mut TestWorld) {
    let (address1, address2) = extract_addresses(world);
    assert_ne!(address1, address2, "Addresses are equal");
}
