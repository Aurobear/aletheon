//! D4 Agora non-authoritative active-workspace port (Agent Kernel V2).
//!
//! Agora becomes a **non-authoritative active workspace**: it holds
//! projection/replay state but never owns core identity or the canonical
//! session/turn truth.  This seam defines the workspace port and the
//! rebuildable projection contract (D4: "证明 projection 可重建和 degraded
//! mode").  No writer cutover; the legacy Executive memory/workspace modules
//! merge at the cutover.

use async_trait::async_trait;

/// A rebuildable Agora workspace projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProjection {
    pub workspace_id: String,
    pub revision: u64,
    /// Empty in degraded mode (available only, not authoritative).
    pub items: Vec<String>,
}

/// Agora active-workspace port.  Non-authoritative: reads/rebuilds only.
#[async_trait]
pub trait ActiveWorkspacePort: Send + Sync {
    /// Rebuild the projection from durable facts (never a second authority).
    async fn rebuild(&self, workspace_id: &str) -> Result<WorkspaceProjection, AgoraError>;
    /// Degraded-mode read: returns an available-only projection with no
    /// authoritative items.
    async fn degraded(&self, workspace_id: &str) -> Result<WorkspaceProjection, AgoraError>;
}

/// Typed Agora errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgoraError {
    #[error("workspace not found")]
    NotFound,
    #[error("workspace source unavailable (degraded)")]
    Unavailable,
}

/// In-memory test port that can rebuild and degrade.
#[derive(Default)]
pub struct InMemoryWorkspacePort;

#[async_trait]
impl ActiveWorkspacePort for InMemoryWorkspacePort {
    async fn rebuild(&self, workspace_id: &str) -> Result<WorkspaceProjection, AgoraError> {
        Ok(WorkspaceProjection {
            workspace_id: workspace_id.to_string(),
            revision: 1,
            items: vec!["item-1".into()],
        })
    }

    async fn degraded(&self, workspace_id: &str) -> Result<WorkspaceProjection, AgoraError> {
        Ok(WorkspaceProjection {
            workspace_id: workspace_id.to_string(),
            revision: 0,
            items: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rebuild_and_degraded_modes() {
        let port = InMemoryWorkspacePort;
        let rebuilt = port.rebuild("ws-1").await.unwrap();
        assert_eq!(rebuilt.revision, 1);
        assert!(!rebuilt.items.is_empty());
        let degraded = port.degraded("ws-1").await.unwrap();
        assert_eq!(degraded.revision, 0);
        assert!(degraded.items.is_empty());
    }
}
