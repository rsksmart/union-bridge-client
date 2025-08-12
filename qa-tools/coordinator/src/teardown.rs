use qa_tools_bitvmx_mock::AutomatedBitVmxMock;
use qa_tools_common::common::kill_process;
use std::process::Child;
use std::sync::Arc;

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

pub fn shutdown_block_indexer(child_opt: Option<Child>) {
    if let Some(mut child) = child_opt {
        println!(
            " *** TEARDOWN *** Shutting down Block Indexer with PID: {}",
            child.id()
        );
        kill_process(&mut child);
    } else {
        println!(" *** TEARDOWN *** No Block Indexer process found, skipping shutdown.");
    }
}

pub fn shutdown_log_indexer(child_opt: Option<Child>) {
    if let Some(mut child) = child_opt {
        println!(
            " *** TEARDOWN *** Shutting down Log Indexer with PID: {}",
            child.id()
        );
        kill_process(&mut child);
    } else {
        println!(" *** TEARDOWN *** No Log Indexer process found, skipping shutdown.");
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
        println!(" *** TEARDOWN *** No Transaction Dispatcher process found, skipping shutdown.");
    }
}

pub fn shutdown_coordinator(child_opt: Option<Child>) {
    if let Some(mut child) = child_opt {
        println!(
            " *** TEARDOWN *** Shutting down Coordinator with PID: {}",
            child.id()
        );
        kill_process(&mut child);
    } else {
        println!(" *** TEARDOWN *** No Coordinator process found, skipping shutdown.");
    }
}

pub fn shutdown_user_api(child: &mut Option<Child>) {
    if let Some(mut process) = child.take() {
        println!("*** TEARDOWN *** Shutting down user-api...");
        let _ = process.kill();
        let _ = process.wait();
        println!("*** TEARDOWN *** User-api stopped");
    }
}

pub async fn shutdown_bitvmx_mock(bitvmx_mock: Option<Arc<AutomatedBitVmxMock>>) {
    if let Some(mut mock) = bitvmx_mock {
        mock.stop().await;
        tokio::task::spawn_blocking(move || drop(mock))
            .await
            .expect("spawn_blocking failed");
    } else {
        println!(" *** TEARDOWN *** No BitVMX mock process found, skipping shutdown.");
    }
}
