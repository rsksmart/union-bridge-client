use check_fork::{check_fork, CheckForkArgs};
use risc0_zkvm::guest::env;

fn main() {
    let args: CheckForkArgs = env::read();

    let output = check_fork(args);

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
