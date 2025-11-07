use thiserror::Error;
use uuid::Uuid;

/// Generic error type for flow operations
#[derive(Error, Debug)]
pub enum FlowError {
    /// Fatal error that requires flow termination
    #[error("Fatal error in flow {flow_id}: {message}")]
    Fatal {
        flow_id: Uuid,
        message: String,
        #[source]
        source: anyhow::Error,
    },

    /// Transient error that could potentially be retried
    #[error("Transient error in flow {flow_id} (attempt {retry_count}): {message}")]
    Transient {
        flow_id: Uuid,
        message: String,
        retry_count: u8,
        #[source]
        source: anyhow::Error,
    },
}

/// Extension trait for Result types to easily convert to FlowError
pub trait FlowResultExt<T> {
    /// Convert any error to a fatal FlowError
    fn or_fail_flow(self, flow_id: Uuid) -> Result<T, FlowError>;

    /// Convert any error to a transient FlowError with retry count
    fn or_retry_flow(self, flow_id: Uuid, retry_count: u8) -> Result<T, FlowError>;
}

impl<T, E> FlowResultExt<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn or_fail_flow(self, flow_id: Uuid) -> Result<T, FlowError> {
        self.map_err(|e| {
            let err = e.into();

            // Try to downcast to FlowError first
            match err.downcast::<FlowError>() {
                Ok(flow_error) => flow_error,
                Err(original_err) => FlowError::Fatal {
                    flow_id,
                    message: original_err.to_string(),
                    source: original_err,
                },
            }
        })
    }

    fn or_retry_flow(self, flow_id: Uuid, retry_count: u8) -> Result<T, FlowError> {
        self.map_err(|e| {
            let err = e.into();

            // Try to downcast to FlowError first
            match err.downcast::<FlowError>() {
                Ok(flow_error) => flow_error,
                Err(original_err) => FlowError::Transient {
                    flow_id,
                    message: original_err.to_string(),
                    retry_count,
                    source: original_err,
                },
            }
        })
    }
}
