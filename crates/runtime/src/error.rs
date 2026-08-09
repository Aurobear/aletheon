//! Runtime typed errors (RA-01).
//!
//! Typed failures — a caller must fail closed on these, never report success
//! on a timeout/EOF/unknown/wrong-generation/provider-rejection.

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("session not found")]
    SessionNotFound,
    #[error("session belongs to a different generation")]
    WrongGeneration,
    #[error("turn is already terminal")]
    AlreadyTerminal,
    #[error("agent run not found")]
    AgentRunNotFound,
    #[error("operation timed out")]
    Timeout,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("unknown or malformed request schema")]
    UnknownSchema,
    #[error("internal runtime error")]
    Internal,
}
