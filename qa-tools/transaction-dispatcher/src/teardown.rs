use qa_tools_common::common::kill_process;
use std::process::Child;

pub fn shutdown_anvil(child_opt: Option<Child>) {
    if let Some(mut child) = child_opt {
        println!(
            " *** TEARDOWN *** Shutting down Anvil with PID: {}",
            child.id()
        );
        kill_process(&mut child);
    } else {
        println!(" *** TEARDOWN *** No Anvil process found, skipping shutdown.");
    }
}

pub fn shutdown_transaction_dispatcher(child_opt: Option<Child>) {
    if let Some(mut child) = child_opt {
        println!(
            " *** TEARDOWN *** Shutting down Transaction Dispatcher with PID: {}",
            child.id()
        );
        kill_process(&mut child);
    } else {
        println!(
            " *** TEARDOWN *** No Transaction Dispatcher process found, skipping shutdown."
        );
    }
}
