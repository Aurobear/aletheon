//! AgoraRegistry — manages per-session Workspaces and implements AgoraOps.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use fabric::AgoraOps;

use crate::workspace::Workspace;

/// Owns one `Workspace` per session id. Cheap to clone via `Arc`.
#[derive(Default)]
pub struct AgoraRegistry {
    sessions: Mutex<HashMap<String, Workspace>>,
}

impl AgoraRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl AgoraOps for AgoraRegistry {
    async fn publish(&self, session: &str, key: &str, value: Value) -> Result<()> {
        let mut map = self.sessions.lock().await;
        let ws = map
            .entry(session.to_string())
            .or_insert_with(|| Workspace::new(session));
        ws.blackboard.set(key, value);
        Ok(())
    }

    async fn recall(&self, session: &str, key: &str) -> Result<Option<Value>> {
        let map = self.sessions.lock().await;
        Ok(map
            .get(session)
            .and_then(|ws| ws.blackboard.get(key).cloned()))
    }

    async fn update(&self, session: &str, patch: Value) -> Result<()> {
        let mut map = self.sessions.lock().await;
        let ws = map
            .entry(session.to_string())
            .or_insert_with(|| Workspace::new(session));
        ws.blackboard.merge(patch);
        Ok(())
    }

    async fn snapshot(&self, session: &str) -> Result<Value> {
        let map = self.sessions.lock().await;
        Ok(map
            .get(session)
            .map(|ws| ws.snapshot())
            .unwrap_or(Value::Null))
    }

    async fn clear(&self, session: &str) -> Result<()> {
        let mut map = self.sessions.lock().await;
        if let Some(ws) = map.get_mut(session) {
            ws.clear();
        }
        Ok(())
    }

    async fn trace(&self, session: &str, kind: &str, content: Value) -> Result<()> {
        let mut map = self.sessions.lock().await;
        let ws = map
            .entry(session.to_string())
            .or_insert_with(|| Workspace::new(session));
        ws.trace.push(kind, content);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn publish_then_recall() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "k", json!("v")).await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), Some(json!("v")));
    }

    #[tokio::test]
    async fn recall_missing_session_is_none() {
        let reg = AgoraRegistry::new();
        assert_eq!(reg.recall("nope", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn update_merges_patch() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "a", json!(1)).await.unwrap();
        reg.update("s1", json!({"b": 2})).await.unwrap();
        assert_eq!(reg.recall("s1", "b").await.unwrap(), Some(json!(2)));
    }

    #[tokio::test]
    async fn snapshot_and_clear() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "k", json!(1)).await.unwrap();
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["blackboard"]["k"], json!(1));
        reg.clear("s1").await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn trace_appends_and_reflects_in_snapshot() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "k", json!(1)).await.unwrap();
        let before = reg.snapshot("s1").await.unwrap();
        assert_eq!(before["trace_len"], json!(0));
        reg.trace("s1", "tool_result", json!({"call_id": "c1", "ok": true}))
            .await
            .unwrap();
        let after = reg.snapshot("s1").await.unwrap();
        assert_eq!(after["trace_len"], json!(1));
    }
}
