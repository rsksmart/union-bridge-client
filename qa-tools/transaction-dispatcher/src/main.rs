mod step_definitions;
mod steps;

use cucumber::{World, writer};
use std::fs::File;
use std::process::Command;
use steps::TestWorld;

async fn setup() {
    match crate::steps::start_anvil().await {
        Ok(_) => (),
        Err(e) => panic!("Failed to start anvil: {}", e),
    }
    // Deploy contracts before transferring ether, otherwise the PegManager address is different to the one hardcoded in the contracts repo scripts
    match crate::steps::execute_script("shell/script/deploy/deploy-local.sh") {
        Ok(_) => (),
        Err(e) => panic!("Failed to deploy local contracts: {}", e),
    }
    match crate::steps::transfer_ether(
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "5bdd03ceaf59cad075cb29c67696581d857b9031",
        "1ether",
    ) {
        Ok(_) => (),
        Err(e) => panic!("Failed to transfer ether: {}", e),
    }
    match crate::steps::start_transaction_dispatcher().await {
        Ok(_) => (),
        Err(e) => panic!("Failed to start transaction dispatcher: {}", e),
    }
}

async fn teardown() {
    let _ = Command::new("pkill").arg("anvil").output();
    let _ = Command::new("pkill").arg("transaction-dispatcher").output();
}

#[tokio::main]
async fn main() {
    let junit_report = std::env::var("JUNIT_REPORT");
    if junit_report.is_ok() {
        let file = File::create(junit_report.unwrap()).unwrap();
        TestWorld::cucumber()
            .init_tracing()
            .with_writer(writer::JUnit::new(file, 0))
            .max_concurrent_scenarios(Some(1)) // Run in sequence to avoid conflicts between scenarios
            .before(|_, _, _, _| Box::pin(setup()))
            .after(|_, _, _, _, _| Box::pin(teardown()))
            .run("transaction-dispatcher/features")
            .await;
    } else {
        TestWorld::cucumber()
            .init_tracing()
            .max_concurrent_scenarios(Some(1)) // Run in sequence to avoid conflicts between scenarios
            .before(|_, _, _, _| Box::pin(setup()))
            .after(|_, _, _, _, _| Box::pin(teardown()))
            .run_and_exit("transaction-dispatcher/features")
            .await;
    }
}
