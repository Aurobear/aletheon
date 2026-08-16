//! Runtime typed errors (RA-01).
//!
//! Typed failures — a caller must fail closed on these, never report success
//! on a timeout/EOF/unknown/wrong-generation/provider-rejection.

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("session not found")]
    SessionNotFound,
    #[error("turn belongs to a different session")]
    WrongSession,
    #[error("session belongs to a different generation")]
    WrongGeneration,
    #[error("runtime writer is retired during maintenance")]
    Retired,
    #[error("maintenance has not drained active runtime writes")]
    MaintenanceNotDrained,
    #[error("turn is already terminal")]
    AlreadyTerminal,
    #[error("agent run not found")]
    AgentRunNotFound,
    #[error("agent run has not reached an authoritative terminal")]
    NotTerminal,
    #[error("turn not found")]
    TurnNotFound,
    #[error("operation timed out")]
    Timeout,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("unknown or malformed request schema")]
    UnknownSchema,
    #[error("request field is not supported by the authoritative runtime writer")]
    UnsupportedRequest,
    #[error("internal runtime error")]
    Internal,
}
