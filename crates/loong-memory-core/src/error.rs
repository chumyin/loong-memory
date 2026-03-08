use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoongMemoryError {
    #[error("validation failed: {0}")]
    Validation(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("not found")]
    NotFound,

    #[error("storage error: {0}")]
    Storage(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
