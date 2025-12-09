use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, Result};
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
    /// # Errors
    ///
    /// Returns an error if the Tokio runtime cannot be created.
    pub fn new() -> Result<Self> {
        // Note: we cannot use Builder::new_current_thread() because Alloy needs multiple to work
        let rt = Runtime::new().context("Failed to create Tokio runtime")?;
        Ok(RuntimeSync { rt: Arc::new(rt) })
    }

    /// # Errors
    ///
    /// Returns an error if the future execution fails.
    pub fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType, Err>
    where
        Fut: Future<Output = Result<RetType, Err>>,
    {
        self.rt.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

    // Custom error type for testing error propagation
    #[derive(Debug, PartialEq)]
    enum TestError {
        CustomError(String),
        AnotherError(i32),
    }

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                TestError::CustomError(msg) => write!(f, "CustomError: {msg}"),
                TestError::AnotherError(code) => write!(f, "AnotherError: {code}"),
            }
        }
    }

    impl std::error::Error for TestError {}

    #[test]
    fn test_runtime_sync_new_succeeds() {
        let rt_sync = RuntimeSync::new();
        assert!(rt_sync.is_ok());
    }

    #[test]
    fn test_run_propagates_success() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result = rt_sync.run(async { Ok::<i32, TestError>(42) });

        assert_eq!(result, Ok(42));
    }

    #[test]
    fn test_run_propagates_custom_error() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result = rt_sync
            .run(async { Err::<i32, TestError>(TestError::CustomError("test error".to_string())) });

        assert_eq!(result, Err(TestError::CustomError("test error".to_string())));
    }

    #[test]
    fn test_run_propagates_different_error_variant() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result = rt_sync.run(async { Err::<String, TestError>(TestError::AnotherError(404)) });

        assert_eq!(result, Err(TestError::AnotherError(404)));
    }

    #[test]
    fn test_run_with_complex_return_type() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result = rt_sync.run(async {
            Ok::<Vec<String>, TestError>(vec!["hello".to_string(), "world".to_string()])
        });

        assert_eq!(result, Ok(vec!["hello".to_string(), "world".to_string()]));
    }

    #[test]
    fn test_run_preserves_error_type_information() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result: Result<i32, TestError> =
            rt_sync.run(async { Err(TestError::CustomError("specific error".to_string())) });

        // Verify we can match on the specific error variant
        match result {
            Err(TestError::CustomError(msg)) => {
                assert_eq!(msg, "specific error");
            }
            _ => panic!("Expected CustomError variant"),
        }
    }

    #[test]
    fn test_run_with_async_computation() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

        let result = rt_sync.run(async {
            // Simulate some async work
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            Ok::<i32, TestError>(100)
        });

        assert_eq!(result, Ok(100));
    }

    #[test]
    fn test_runtime_sync_is_cloneable() {
        let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");
        let rt_sync_clone = rt_sync.clone();

        let result1 = rt_sync.run(async { Ok::<i32, TestError>(1) });
        let result2 = rt_sync_clone.run(async { Ok::<i32, TestError>(2) });

        assert_eq!(result1, Ok(1));
        assert_eq!(result2, Ok(2));
    }
}
