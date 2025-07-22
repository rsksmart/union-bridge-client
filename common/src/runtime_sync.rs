use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use tokio::runtime::Runtime;

// This struct is a wrapper around tokio::runtime::Runtime that allows for synchronous execution of
// async functions.
// Note 1: it is discouraged to start several runtimes, so use with caution.
// Note 2: we need Tokio because Alloy requires a Tokio Reactor to work
#[derive(Clone)]
pub struct RuntimeSync {
    rt: Arc<Runtime>,
}

impl RuntimeSync {
    pub fn new() -> Result<Self> {
        // Note: we cannot use Builder::new_current_thread() because Alloy needs multiple to work
        let rt = Runtime::new().context("Failed to create Tokio runtime")?;
        Ok(RuntimeSync { rt: Arc::new(rt) })
    }

    pub fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output = Result<RetType, Err>>,
        Err: std::error::Error + Send + 'static,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error on RuntimeSync: {:?}", e))
                .context("Async operation failed")
        })
    }
}
