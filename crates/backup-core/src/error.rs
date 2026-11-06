use std::time::Duration;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("precondition failed: {0}")]
    FailedPrecondition(String),
    #[error("busy: {0}")]
    Busy(String),
    #[error("{operation} timed out after {limit:?}")]
    Timeout { operation: String, limit: Duration },
    #[error("storage error: {0}")]
    Storage(String),
    #[error("`{program}` failed with {status}: {stderr}")]
    Process {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
