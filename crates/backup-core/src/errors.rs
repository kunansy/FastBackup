#[derive(thiserror::Error, Debug, Clone)]
pub enum Errors {
    #[error("Could not init with env: {0}")]
    EnvError(String),
    #[error("Could not dump the database: {0}")]
    DumpError(String),
    #[error("Error with the storage: {0}")]
    StorageError(String),
}

impl From<std::env::VarError> for Errors {
    fn from(value: std::env::VarError) -> Self {
        Self::EnvError(value.to_string())
    }
}

impl From<std::io::Error> for Errors {
    fn from(value: std::io::Error) -> Self {
        Self::DumpError(value.to_string())
    }
}
