#[cfg(test)]
use mockall::automock;
use thiserror::Error;

#[cfg_attr(test, automock)]
pub trait FailableFlow {
    fn fail(&mut self) -> ();
}

/// Generic error type for flow operations
#[derive(Error, Debug)]
pub enum FlowError {
    /// Fatal error that requires flow termination
    #[error("Fatal error: {message}")]
    Fatal {
        message: String,
        #[source]
        source: Option<anyhow::Error>,
    },

    /// Transient error that could potentially be retried
    #[error("Transient error: {message}")]
    Transient {
        message: String,
        #[source]
        source: Option<anyhow::Error>,
    },
}

impl FlowError {
    /// Create a transient error without a source
    pub fn transient(message: impl Into<String>) -> Self {
        FlowError::Transient { message: message.into(), source: None }
    }
}

/// Extension trait for Result types to easily convert to `FlowError`
pub trait FlowResultExt<T> {
    /// Convert any error to a fatal `FlowError`
    #[allow(unused)]
    fn or_fatal(self) -> Result<T, FlowError>;

    /// Convert any error to a transient `FlowError`
    fn or_transient(self) -> Result<T, FlowError>;
}

impl<T, E> FlowResultExt<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn or_fatal(self) -> Result<T, FlowError> {
        self.map_err(|e| {
            let anyhow_err: anyhow::Error = e.into();
            anyhow_err.into() // uses From<anyhow::Error> which defaults to Fatal
        })
    }

    fn or_transient(self) -> Result<T, FlowError> {
        self.map_err(|e| {
            let anyhow_err: anyhow::Error = e.into();
            // always convert to Transient, even if it's already a FlowError
            FlowError::transient(anyhow_err.to_string())
        })
    }
}

impl From<anyhow::Error> for FlowError {
    fn from(err: anyhow::Error) -> Self {
        // try to downcast to FlowError first, otherwise default to Fatal
        err.downcast::<FlowError>().unwrap_or_else(|original_err| FlowError::Fatal {
            message: original_err.to_string(),
            source: Some(original_err),
        })
    }
}
