use anyhow::{anyhow, Result};
use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::runtime::Runtime;

/**
 * This struct is a wrapper around tokio::runtime::Runtime that allows for
 * synchronous execution of async functions. Note: it is discouraged to start
 * several runtimes, so use with caution.
 */
pub struct RuntimeSync {
    rt: Runtime,
}

impl RuntimeSync {
    pub fn new() -> Result<Self> {
        let rt = Runtime::new().expect("Failed to create Tokio runtime (unrecoverable)");
        Ok(RuntimeSync { rt })
    }

    pub fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output = Result<RetType, Err>>,
        RetType: Send + 'static,
        Err: Error + Send + 'static,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error on RuntimeSync: {:?}", e))
        })
    }
}

#[derive(Clone)]
pub struct ShutdownFlag {
    flag: Arc<AtomicBool>,
}

impl ShutdownFlag {
    pub fn init() -> Self {
        ShutdownFlag {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_on(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_on(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}
