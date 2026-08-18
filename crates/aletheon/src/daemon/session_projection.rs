//! Typed Gateway session projection backed by the canonical SessionService.
//!
//! This adapter owns no store or writer. It receives the single composed
//! SessionService and exposes only principal-scoped snapshot/list/event reads.

use std::sync::Arc;

use runtime::session_service::SessionService;
use tokio::sync::Mutex;

pub(crate) struct CanonicalSessionProjection {
    sessions: Arc<SessionService>,
}

impl CanonicalSessionProjection {
    pub(crate) fn new(sessions: Arc<SessionService>) -> Self {
        Self { sessions }
    }
}

#[async_trait::async_trait]
impl super::handler::ports::SessionProjectionPort for CanonicalSessionProjection {
    async fn protocol_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::UiSnapshot> {
        self.sessions
            .protocol_snapshot_for(principal, session_id)
            .await
    }

    async fn protocol_snapshot(
        &self,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::UiSnapshot> {
        self.sessions.protocol_snapshot(session_id).await
    }

    async fn protocol_read_snapshot(
        &self,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot> {
        self.sessions.protocol_read_snapshot(session_id).await
    }

    async fn protocol_read_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot> {
        self.sessions
            .protocol_read_snapshot_for(principal, session_id)
            .await
    }

    async fn protocol_event_page_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> anyhow::Result<::contracts::protocol::client::SessionEventPage> {
        self.sessions
            .protocol_event_page_for(principal, session_id, after)
            .await
    }

    async fn protocol_session_list_for(
        &self,
        principal: &::contracts::PrincipalId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionListSnapshot> {
        self.sessions.protocol_session_list_for(principal).await
    }

    async fn protocol_events_after_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> anyhow::Result<Vec<::contracts::protocol::client::ClientEvent>> {
        self.sessions
            .protocol_events_after_for(principal, session_id, after)
            .await
    }
}

/// Typed memory read adapter backed by the already composed memory stores.
/// It owns neither store lifecycle nor persistence and returns the same bounded
/// projection shape as the retired `session.memory` compatibility method.
pub(crate) struct CanonicalSessionMemoryProjection {
    core: Arc<Mutex<mnemosyne::runtime::CoreMemory>>,
    recall: Arc<Mutex<mnemosyne::runtime::RecallMemory>>,
}

impl CanonicalSessionMemoryProjection {
    pub(crate) fn new(
        core: Arc<Mutex<mnemosyne::runtime::CoreMemory>>,
        recall: Arc<Mutex<mnemosyne::runtime::RecallMemory>>,
    ) -> Self {
        Self { core, recall }
    }
}

#[async_trait::async_trait]
impl super::handler::ports::SessionMemoryProjectionPort for CanonicalSessionMemoryProjection {
    async fn snapshot(&self, memory_type: &str, limit: usize) -> anyhow::Result<serde_json::Value> {
        anyhow::ensure!(
            matches!(memory_type, "all" | "core" | "recall"),
            "memory_type must be all, core, or recall"
        );
        anyhow::ensure!(
            (1..=256).contains(&limit),
            "memory snapshot limit must be between 1 and 256"
        );

        let mut content = String::from("# Memory\n\n");
        if matches!(memory_type, "all" | "core") {
            let core = self.core.lock().await;
            content.push_str("## Core Memory Blocks\n\n");
            for (label, block) in core.blocks() {
                use std::fmt::Write as _;
                let _ = write!(
                    content,
                    "### {label}\n- char_limit: {}\n- read_only: {}\n\n{}\n\n",
                    block.char_limit, block.read_only, block.value
                );
            }
        }
        if matches!(memory_type, "all" | "recall") {
            let recall = self.recall.lock().await;
            content.push_str("## Recall Memory (Recent)\n\n");
            let entries = recall.recent(limit)?;
            if entries.is_empty() {
                content.push_str("*(no entries)*\n");
            } else {
                for entry in entries {
                    use std::fmt::Write as _;
                    let _ = writeln!(content, "- **[{}]** {}", entry.entry_type, entry.content);
                }
            }
            content.push('\n');
        }

        Ok(serde_json::json!({
            "memory_type": memory_type,
            "content": content,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::handler::ports::SessionMemoryProjectionPort;

    fn projection(
        dir: &tempfile::TempDir,
    ) -> (
        CanonicalSessionMemoryProjection,
        Arc<Mutex<mnemosyne::runtime::RecallMemory>>,
    ) {
        let clock: Arc<dyn ::contracts::Clock> = Arc::new(kernel::chronos::SystemClock::new());
        let core = Arc::new(Mutex::new(mnemosyne::runtime::CoreMemory::with_defaults()));
        let recall = Arc::new(Mutex::new(
            mnemosyne::runtime::RecallMemory::new(&dir.path().join("recall.db"), clock).unwrap(),
        ));
        (
            CanonicalSessionMemoryProjection::new(core, recall.clone()),
            recall,
        )
    }

    #[tokio::test]
    async fn typed_memory_projection_reads_the_composed_core_and_recall_stores() {
        let dir = tempfile::tempdir().unwrap();
        let (projection, recall) = projection(&dir);
        recall
            .lock()
            .await
            .store("session-1", "fact", "typed recall evidence", None)
            .unwrap();

        let snapshot = projection.snapshot("all", 20).await.unwrap();
        assert_eq!(snapshot["memory_type"], "all");
        let content = snapshot["content"].as_str().unwrap();
        assert!(content.contains("## Core Memory Blocks"));
        assert!(content.contains("typed recall evidence"));
    }

    #[tokio::test]
    async fn typed_memory_projection_fails_closed_for_unbounded_or_unknown_queries() {
        let dir = tempfile::tempdir().unwrap();
        let (projection, _) = projection(&dir);
        assert!(projection.snapshot("unknown", 20).await.is_err());
        assert!(projection.snapshot("recall", 0).await.is_err());
        assert!(projection.snapshot("recall", 257).await.is_err());
    }
}
