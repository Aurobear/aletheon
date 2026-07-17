//! Prompt queue + interjection types (G3).
//!
//! A session `(principal, thread)` owns a versioned queue of pending prompts
//! and a mid-turn interjection buffer. This module holds the pure types plus
//! the optimistic-concurrency edit/cancel rules; the coordinator, persistence,
//! and turn-loop safe-point draining live in the Executive.
//!
//! See `docs/plans/grok/exec/G3-prompt-queue.md`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::admission::PrincipalId;
use crate::types::local_authority::{ConnectionId, ThreadId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptId(pub Uuid);

impl PromptId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PromptId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKind {
    Prompt,
    Interjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptState {
    Queued,
    Running,
    Completed,
    Cancelled,
    Rejected,
}

/// Unified envelope for queued prompts and interjections. The persisted unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptEnvelope {
    pub prompt_id: PromptId,
    /// Monotonic version; edit/cancel must carry the expected version.
    pub version: u64,
    /// Owner; never changes on edit.
    pub principal_id: PrincipalId,
    /// Last-editor source connection.
    pub connection_id: ConnectionId,
    pub thread_id: ThreadId,
    pub kind: PromptKind,
    /// Bounded; over-limit content is truncated (UTF-8 safe) before storage.
    pub content: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub state: PromptState,
    /// Idempotency key for replay / reconnect dedup.
    pub idempotency_key: String,
}

/// Result of an optimistic edit/cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueOpResult {
    Ok {
        new_version: u64,
    },
    /// Expected version stale; carries current envelope for client rebase.
    Conflict {
        current: PromptEnvelope,
    },
    /// Cross-principal edit, or in-place edit of a running prompt, etc.
    Rejected {
        reason: String,
    },
}

/// Queue bounds.
pub const MAX_QUEUE_LEN: usize = 64;
pub const MAX_PROMPT_BYTES: usize = 128 * 1024;
pub const MAX_INTERJECTION_BYTES: usize = 16 * 1024;

/// Pure edit rule (optimistic concurrency + authority checks). The coordinator
/// calls this before persisting; keeping it pure makes the concurrency
/// semantics fully unit-testable without a store.
///
/// Rules:
/// - editor principal must equal the owner (cross-principal edit rejected);
/// - a Running prompt cannot be edited in place (convert to interjection or
///   enqueue-next instead);
/// - the expected version must match the current version, else Conflict;
/// - on success the version is bumped and the owner is preserved.
pub fn evaluate_edit(
    current: &PromptEnvelope,
    editor_principal: &PrincipalId,
    editor_connection: ConnectionId,
    expected_version: u64,
    new_content: &str,
) -> QueueOpResult {
    if *editor_principal != current.principal_id {
        return QueueOpResult::Rejected {
            reason: "cross-principal edit is not permitted".to_string(),
        };
    }
    if current.state == PromptState::Running {
        return QueueOpResult::Rejected {
            reason: "running prompt cannot be edited in place".to_string(),
        };
    }
    if expected_version != current.version {
        return QueueOpResult::Conflict {
            current: current.clone(),
        };
    }
    let _ = (editor_connection, new_content);
    QueueOpResult::Ok {
        new_version: current.version + 1,
    }
}

/// Pure cancel rule: cross-principal cancel rejected; version must match.
pub fn evaluate_cancel(
    current: &PromptEnvelope,
    requester: &PrincipalId,
    expected_version: u64,
) -> QueueOpResult {
    if *requester != current.principal_id {
        return QueueOpResult::Rejected {
            reason: "cross-principal cancel is not permitted".to_string(),
        };
    }
    if expected_version != current.version {
        return QueueOpResult::Conflict {
            current: current.clone(),
        };
    }
    QueueOpResult::Ok {
        new_version: current.version + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> PromptEnvelope {
        PromptEnvelope {
            prompt_id: PromptId::new(),
            version: 3,
            principal_id: PrincipalId("local-uid:1000".to_string()),
            connection_id: ConnectionId(Uuid::nil()),
            thread_id: ThreadId("t1".to_string()),
            kind: PromptKind::Prompt,
            content: "hello".to_string(),
            created_at_unix: 1,
            updated_at_unix: 1,
            state: PromptState::Queued,
            idempotency_key: "k1".to_string(),
        }
    }

    #[test]
    fn edit_matching_version_bumps() {
        let e = env();
        let r = evaluate_edit(
            &e,
            &e.principal_id.clone(),
            ConnectionId(Uuid::nil()),
            3,
            "new",
        );
        assert_eq!(r, QueueOpResult::Ok { new_version: 4 });
    }

    #[test]
    fn edit_stale_version_conflicts() {
        let e = env();
        let r = evaluate_edit(
            &e,
            &e.principal_id.clone(),
            ConnectionId(Uuid::nil()),
            2, // stale
            "new",
        );
        assert!(matches!(r, QueueOpResult::Conflict { .. }));
    }

    #[test]
    fn cross_principal_edit_rejected() {
        let e = env();
        let other = PrincipalId("local-uid:2000".to_string());
        let r = evaluate_edit(&e, &other, ConnectionId(Uuid::nil()), 3, "new");
        assert!(matches!(r, QueueOpResult::Rejected { .. }));
    }

    #[test]
    fn running_prompt_cannot_be_edited() {
        let mut e = env();
        e.state = PromptState::Running;
        let r = evaluate_edit(
            &e,
            &e.principal_id.clone(),
            ConnectionId(Uuid::nil()),
            3,
            "new",
        );
        assert!(matches!(r, QueueOpResult::Rejected { .. }));
    }

    #[test]
    fn cancel_matching_version_ok() {
        let e = env();
        let r = evaluate_cancel(&e, &e.principal_id.clone(), 3);
        assert_eq!(r, QueueOpResult::Ok { new_version: 4 });
    }

    #[test]
    fn cross_principal_cancel_rejected() {
        let e = env();
        let other = PrincipalId("local-uid:2000".to_string());
        let r = evaluate_cancel(&e, &other, 3);
        assert!(matches!(r, QueueOpResult::Rejected { .. }));
    }

    #[test]
    fn bounds_are_sane() {
        const { assert!(MAX_INTERJECTION_BYTES < MAX_PROMPT_BYTES) };
        const { assert!(MAX_QUEUE_LEN >= 8) };
    }

    #[test]
    fn envelope_serde_roundtrip() {
        let e = env();
        let json = serde_json::to_string(&e).unwrap();
        let back: PromptEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
