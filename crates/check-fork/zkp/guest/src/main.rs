#![forbid(unsafe_code)]

use check_fork::{CheckForkArgs, build_check_fork_journal, check_fork, compute_pegout_id};
use risc0_zkvm::guest::env;

fn main() {
    let args: Vec<u8> = env::read();
    let args_des: CheckForkArgs = bincode::deserialize(&args).expect("Failed to deserialize args");
    let pegout_id = compute_pegout_id(&args_des);

    let output = check_fork(&args_des, pegout_id);

    let accepted = match output {
        Ok(effort) => {
            println!("Guest output: ACCEPT, check_fork effort: {effort}");
            true
        }
        Err(e) => {
            println!("Guest output: REJECT, check_fork error: {e}");
            false
        }
    };

    let journal = build_check_fork_journal(&args_des, pegout_id, accepted).to_bytes();
    env::commit_slice(&journal);
}
