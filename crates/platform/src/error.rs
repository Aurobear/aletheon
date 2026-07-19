//! Platform-agnostic error types.

/// Stable host error kinds. Every backend maps its native errors here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostErrorKind {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("cancelled: {0}")]
    Cancelled(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("resource exhausted: {0}")]
    ResourceExhausted(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
#[error("{kind}: {detail}")]
pub struct HostError {
    pub kind: HostErrorKind,
    pub detail: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl HostError {
    pub fn new(kind: HostErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            source: None,
        }
    }

    pub fn unsupported(feature: impl Into<String>) -> Self {
        let feature = feature.into();
        Self::new(HostErrorKind::Unsupported(feature.clone()), feature)
    }
}
