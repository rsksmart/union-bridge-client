use check_fork::{check_fork, CheckForkArgs};
use risc0_zkvm::guest::env;

fn main() {
    let args: Vec<u8> = env::read();
    let args_des: CheckForkArgs = bincode::deserialize(&args).expect("Failed to deserialize args");

    let output = check_fork(args_des);

    let result = match output {
        Ok(effort) => {
            println!("Guest output: ACCEPT, check_fork effort: {}", effort);
            0
        }
        Err(e) => {
            println!("Guest output: REJECT, check_fork error: {}", e);
            1
        }
        // TODO competing fork, should return 2 when implemented
    };

    env::commit(&result);
}
