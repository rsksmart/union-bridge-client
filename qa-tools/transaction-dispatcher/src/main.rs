mod step_definitions;
mod steps;

use cucumber::{World, writer};
use std::fs::File;
use std::process::Command;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use steps::TestWorld;

static TRANSACTION_DISPATCHER_PID: Lazy<Mutex<Option<u32>>> = Lazy::new(|| Mutex::new(None));

async fn setup() {
    // let _ = Command::new("pkill").arg("anvil").output();
    // let output_ps = Command::new("sh")
    //     .arg("-c")
    //     .arg("ps -ef | grep transaction-dispatcher")
    //     .output();
    // println!(
    //     "output_ps output: {:?}", output_ps
    // );
    // 
    // // let output = Command::new("pkill").arg("transaction-dispatcher").output().expect("Failed to kill transaction-dispatcher");
    // // println!(
    // //     "Killed transaction-dispatcher: {:?}", output
    // // );


    match crate::steps::start_anvil().await {
        Ok(_) => (),
        Err(e) => panic!("Failed to start anvil: {}", e),
    }
    // Deploy contracts before transferring ether, otherwise the PegManager address is different to the one hardcoded in the contracts repo scripts
    match crate::steps::execute_script("shell/script/deploy/deploy-local.sh") {
        Ok(_) => (),
        Err(e) => panic!("Failed to deploy local contracts: {}", e),
    }

    // 3. Verify deployment by checking on-chain code
    let check_contract_cmd = format!(
        "curl -s -X POST -H 'Content-Type: application/json' --data '{{\"jsonrpc\":\"2.0\",\"method\":\"eth_getCode\",\"params\":[\"{}\",\"latest\"],\"id\":1}}' http://localhost:9385",
        "0x0165878A594ca255338adfa4d48449f69242Eb8F"
    );

    let output = Command::new("sh")
        .arg("-c")
        .arg(check_contract_cmd)
        .output()
        .expect("Failed to execute code check");

    println!("Contract code response: {}", String::from_utf8_lossy(&output.stdout));

    match crate::steps::transfer_ether(
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "0x5bdd03ceaf59cad075cb29c67696581d857b9031",
        "1ether",
    ) {
        Ok(_) => (),
        Err(e) => panic!("Failed to transfer ether: {}", e),
    }

    // Verify the transfer by checking the balance
    let verify_balance_cmd = format!(
        "curl -s -X POST -H 'Content-Type: application/json' --data '{{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBalance\",\"params\":[\"{}\",\"latest\"],\"id\":1}}' http://localhost:9385 | jq -r '.result'",
        "0x5bdd03ceaf59cad075cb29c67696581d857b9031"
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(verify_balance_cmd)
        .output()
        .expect("Failed to execute verify-balance command");
    // Print the output
    println!("Balance: {}", String::from_utf8_lossy(&output.stdout));

    match crate::steps::start_transaction_dispatcher().await {
        Ok((message, pid)) => {
            println!("{}", message);
            if let Some(pid_value) = pid {
                println!("Transaction dispatcher PID: {}", pid_value);
                *TRANSACTION_DISPATCHER_PID.lock().unwrap() = Some(pid_value);
            }
        },
        Err(e) => panic!("Failed to start transaction dispatcher: {}", e),
    }
}

async fn teardown() {
    println!("================= Teardown ===================");
    
    let _ = Command::new("pkill").arg("anvil").output();
    
    // Get the PID and handle potential errors
    let pid_result = TRANSACTION_DISPATCHER_PID.lock().unwrap().take();
    match pid_result {
        Some(pid) => {
            println!("Attempting to kill transaction dispatcher with PID: {}", pid);
            
            // Check if process is still running
            let check_cmd = Command::new("ps")
                .arg("-p")
                .arg(pid.to_string())
                .output();
            
            match check_cmd {
                Ok(output) => {
                    if output.status.success() {
                        println!("Process {} is still running, killing it...", pid);
                        println!("About to execute kill command...");
                        
                        // First try graceful termination with SIGTERM
                        let term_result = Command::new("kill")
                            .arg("-TERM")
                            .arg(pid.to_string())
                            .output();
                        
                        match term_result {
                            Ok(_) => {
                                println!("Sent SIGTERM to process {}", pid);
                                // Wait a bit for graceful shutdown
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                
                                // Check if process is still running
                                let check_again = Command::new("ps")
                                    .arg("-p")
                                    .arg(pid.to_string())
                                    .output();
                                
                                if let Ok(check_output) = check_again {
                                    if check_output.status.success() {
                                        println!("Process still running, sending SIGKILL...");
                                        let kill_result = Command::new("kill")
                                            .arg("-9")
                                            .arg(pid.to_string())
                                            .output();
                                        
                                        match kill_result {
                                            Ok(kill_output) => {
                                                println!("Kill command completed with status: {:?}", kill_output.status);
                                                println!("Kill command success check: {}", kill_output.status.success());
                                                if kill_output.status.success() {
                                                    println!("Successfully killed transaction dispatcher with PID: {}", pid);
                                                } else {
                                                    println!("Failed to kill process {}: {:?}", pid, kill_output);
                                                }
                                            }
                                            Err(e) => println!("Error executing kill command: {}", e),
                                        }
                                    } else {
                                        println!("Process {} terminated gracefully with SIGTERM", pid);
                                    }
                                }
                            }
                            Err(e) => println!("Error sending SIGTERM: {}", e),
                        }
                    } else {
                        println!("Process {} is no longer running", pid);
                    }
                }
                Err(e) => println!("Error checking if process {} is running: {}", pid, e),
            }
        }
        None => {
            println!("No transaction dispatcher PID found, using pkill fallback");
            let _ = Command::new("pkill").arg("transaction-dispatcher").output();
        }
    }
    
    // Additional port cleanup - kill any process using port 3000
    println!("Killing any process using port 3000...");
    
    // Use ps and grep to find processes that might be using port 3000
    // This is a more basic approach that should work in most environments
    let ps_result = Command::new("ps")
        .arg("aux")
        .output();
    
    if let Ok(output) = ps_result {
        let output_str = String::from_utf8_lossy(&output.stdout);
        for line in output_str.lines() {
            // Look for transaction-dispatcher processes
            // We want to kill the actual service, not the test runner
            if line.contains("transaction-dispatcher") && 
               !line.contains("grep") && 
               !line.contains("qa-tools-transaction-dispatcher") &&
               (line.contains("--config-path") || line.contains("--bin transaction-dispatcher")) {
                
                // Extract PID from ps output (second column)
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    if let Ok(pid) = parts[1].parse::<u32>() {
                        println!("Found transaction-dispatcher process to kill: PID {} - Command: {}", pid, line);
                        let _ = Command::new("kill")
                            .arg("-9")
                            .arg(pid.to_string())
                            .output();
                        println!("Killed transaction-dispatcher process {} (via ps)", pid);
                    }
                }
            }
        }
    }
    
    // Also try pkill as a backup, but be more specific
    println!("Executing pkill -f transaction-dispatcher (excluding test runner)...");
    let _ = Command::new("pkill")
        .arg("-f")
        .arg("transaction-dispatcher --config-path")
        .output();
    println!("Executed pkill -f transaction-dispatcher --config-path");
    
    // Wait longer to ensure port is fully released
    println!("Waiting for port to be released...");
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
