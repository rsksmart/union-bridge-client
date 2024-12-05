use check_fork::{check_fork, CheckForkArgs};
use risc0_zkvm::guest::env;

fn main() {
    let args: Vec<u8> = env::read();
    let args_des: CheckForkArgs = bincode::deserialize(&args).expect("Failed to deserialize args");

    let output = check_fork(args_des);

    let result = match output {
        Ok(effort) => {
            println!("Guest output: ACCEPT, check_fork effort: {}", effort);
            "ACCEPT"
        }
        Err(e) => {
            println!("Guest output: REJECT, check_fork error: {}", e);
            "REJECT"
        }
    };

    env::commit(&result);
}
