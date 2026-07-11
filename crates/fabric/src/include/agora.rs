//! Agora (working-memory) trait contract — the shared cognitive workspace.
//!
//! Like the other subsystem contracts in `include/`, this defines the interface
//! Executive and the cognitive subsystems use to read/write the session-scoped
//! blackboard. The implementation lives in the `agora` crate (`AgoraRegistry`).
//!
//! Session-scoped, in-memory. Persists only via `snapshot()` → Mnemosyne.

use anyhow::Result;
use async_trait::async_trait;

use crate::primitives::cognitive::Evidence;

/// Agora (working-memory) operations — the shared cognitive workspace.
#[async_trait]
pub trait AgoraOps: Send + Sync {
    /// Write a value onto a session's blackboard.
    async fn publish(&self, session: &str, key: &str, value: serde_json::Value) -> Result<()>;
    /// Read a value from a session's blackboard.
    async fn recall(&self, session: &str, key: &str) -> Result<Option<serde_json::Value>>;
    /// Merge a JSON patch into the session workspace.
    async fn update(&self, session: &str, patch: serde_json::Value) -> Result<()>;
    /// Snapshot the entire session workspace (for debug / commit).
    async fn snapshot(&self, session: &str) -> Result<serde_json::Value>;
    /// Clear a session's workspace.
    async fn clear(&self, session: &str) -> Result<()>;
    /// Append an entry onto a session's reasoning trace.
    async fn trace(&self, session: &str, kind: &str, content: serde_json::Value) -> Result<()>;

    // -- Typed vocabulary (RFC-017) layered over the generic trace --------
    //
    // These default methods lower a cognitive primitive onto the untyped
    // trace so producers speak the RFC-017 vocabulary instead of hand-rolled
    // JSON. Reading them back (via `snapshot`) deserializes into the same
    // type. Add more recorders here as real producers for other primitives
    // (Hypothesis, Narrative, …) appear — not before (YAGNI).

    /// Record a typed [`Evidence`] onto the session's reasoning trace
    /// (trace kind `"evidence"`).
    async fn record_evidence(&self, session: &str, evidence: &Evidence) -> Result<()> {
        let content = serde_json::to_value(evidence)?;
        self.trace(session, "evidence", content).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Captures the last `trace()` call so we can assert `record_evidence`
    /// lowers correctly and round-trips the typed object.
    #[derive(Default)]
    struct SpyAgora {
        last: Mutex<Option<(String, String, serde_json::Value)>>,
    }

    #[async_trait]
    impl AgoraOps for SpyAgora {
        async fn publish(&self, _: &str, _: &str, _: serde_json::Value) -> Result<()> {
            Ok(())
        }
        async fn recall(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>> {
            Ok(None)
        }
        async fn update(&self, _: &str, _: serde_json::Value) -> Result<()> {
            Ok(())
        }
        async fn snapshot(&self, _: &str) -> Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
        async fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn trace(&self, session: &str, kind: &str, content: serde_json::Value) -> Result<()> {
            *self.last.lock().unwrap() = Some((session.into(), kind.into(), content));
            Ok(())
        }
    }

    #[tokio::test]
    async fn record_evidence_lowers_to_trace_and_roundtrips() {
        let spy = SpyAgora::default();
        let ev = Evidence::from_tool_result("c1", "bash", "exit 0", false);
        spy.record_evidence("s1", &ev).await.unwrap();

        let (session, kind, content) = spy.last.lock().unwrap().clone().unwrap();
        assert_eq!(session, "s1");
        assert_eq!(kind, "evidence");

        // Consumer half: the trace content deserializes back into Evidence.
        let back: Evidence = serde_json::from_value(content).unwrap();
        assert_eq!(back.id, "c1");
        assert_eq!(back.source, "bash");
        assert_eq!(back.weight, 1.0);
    }
}
