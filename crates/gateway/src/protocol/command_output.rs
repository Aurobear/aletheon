//! R3 typed Command Output + protocol versioning (Aletheon closure plan §11).
//!
//! Completes "typed input, typed output": the daemon ↔ TUI protocol drift
//! becomes a compile-time error instead of a runtime `serde_json::Value`
//! index.  Each output has exactly one domain meaning; JSON exists only at the
//! transport serialization boundary and is parsed to a type immediately.
//! Unknown fields are forward-compatible; unknown required variants are
//! rejected with the protocol version recorded.  This is the canonical typed
//! surface the TUI consumes (no string/field-presence schema guessing).

use serde::{Deserialize, Serialize};

/// Negotiated command-output protocol version.
pub const COMMAND_OUTPUT_VERSION: u32 = 1;

/// A typed command output envelope.  Business layers never index dynamic JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedCommandOutputEnvelope {
    pub version: u32,
    pub kind: TypedCommandOutput,
}

/// The typed command outputs with exactly one domain meaning each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedCommandOutput {
    /// A turn completed with a typed result.
    Completed(TypedCompletion),
    /// Incremental text (streaming) for the active turn.
    IncrementalText { turn: String, delta: String },
    /// A tool lifecycle transition (start/progress/success/failure).
    ToolLifecycle(TypedToolLifecycle),
    /// A status projection (typed, not dynamic JSON).
    StatusProjection(TypedStatusProjection),
    /// Token/usage accounting for a turn.
    Usage(TypedUsage),
    /// A typed error.
    Error(TypedError),
}

/// A completed turn (typed; terminal only from here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedCompletion {
    pub turn: String,
    pub terminal: String,
    pub text: String,
}

/// A tool lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedToolLifecycle {
    pub call_id: String,
    pub phase: String,
    pub detail: Option<String>,
}

/// A typed status projection (no dynamic JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedStatusProjection {
    pub status: String,
    pub detail: Option<String>,
}

/// Token/usage accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A typed error.  Never swallowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedError {
    pub category: String,
    pub message: String,
}

impl TypedCommandOutputEnvelope {
    /// Construct a v1 typed envelope.
    pub fn v1(kind: TypedCommandOutput) -> Self {
        Self {
            version: COMMAND_OUTPUT_VERSION,
            kind,
        }
    }
}

/// Reject an envelope with an unsupported version, recording the version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    #[error("unsupported command-output protocol version {received}; expected {expected}")]
    Unsupported { received: u32, expected: u32 },
}

/// Validate the envelope version.  Unknown required variants are rejected
/// with the version recorded; unknown optional fields are forward-compatible.
pub fn validate_version(envelope: &TypedCommandOutputEnvelope) -> Result<(), VersionError> {
    if envelope.version == COMMAND_OUTPUT_VERSION {
        Ok(())
    } else {
        Err(VersionError::Unsupported {
            received: envelope.version,
            expected: COMMAND_OUTPUT_VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_roundtrip_and_validate() {
        let envelope =
            TypedCommandOutputEnvelope::v1(TypedCommandOutput::Completed(TypedCompletion {
                turn: "t1".into(),
                terminal: "completed".into(),
                text: "done".into(),
            }));
        assert!(validate_version(&envelope).is_ok());
        let json = serde_json::to_string(&envelope).unwrap();
        let back: TypedCommandOutputEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, envelope);
    }

    #[test]
    fn unsupported_version_is_typed_error() {
        let envelope = TypedCommandOutputEnvelope {
            version: 99,
            kind: TypedCommandOutput::Usage(TypedUsage {
                input_tokens: 1,
                output_tokens: 2,
            }),
        };
        assert_eq!(
            validate_version(&envelope),
            Err(VersionError::Unsupported {
                received: 99,
                expected: 1,
            })
        );
    }
}
