//! Application typed errors (APX-01).

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("session not found")]
    SessionNotFound,
    #[error("session already exists")]
    SessionAlreadyExists,
    #[error("invalid session reference")]
    InvalidSessionReference,
    #[error("runtime rejected the request: {0}")]
    RuntimeRejected(String),
    #[error("unknown use case")]
    UnknownUseCase,
}
