use check_fork::{CheckForkArgs, build_check_fork_journal_from_args, check_fork};
use risc0_zkvm::guest::env;

fn main() {
    let args: Vec<u8> = env::read();
    let args_des: CheckForkArgs = bincode::deserialize(&args).expect("Failed to deserialize args");

    let output = check_fork(&args_des);

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

    let journal = build_check_fork_journal_from_args(&args_des, accepted).to_bytes();
    env::commit_slice(&journal);
}
