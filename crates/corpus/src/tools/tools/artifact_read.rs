//! Bounded retrieval for content-addressed tool-output artifacts.

use async_trait::async_trait;
use serde_json::json;

use crate::tools::artifact::ArtifactStore;

use super::{PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult, ToolResultMeta};

const DEFAULT_LIMIT: u64 = 64 * 1024;
const MAX_LIMIT: u64 = 256 * 1024;

#[derive(Clone, Default)]
pub struct ArtifactReadTool {
    root: Option<std::path::PathBuf>,
}

impl ArtifactReadTool {
    fn store(&self) -> ArtifactStore {
        ArtifactStore::new(self.root.clone().unwrap_or_else(|| {
            super::output::OutputConfig::default()
                .overflow_dir
                .join("artifacts")
        }))
    }
}

#[async_trait]
impl Tool for ArtifactReadTool {
    fn name(&self) -> &str {
        "artifact_read"
    }

    fn description(&self) -> &str {
        "Read a bounded byte range from a content-addressed artifact_ref returned by another tool."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_ref": {
                    "type": "string",
                    "description": "artifact://sha256/<digest> or the 64-character SHA-256 digest"
                },
                "offset": {"type": "integer", "minimum": 0},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT}
            },
            "required": ["artifact_ref"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn cache_policy(&self) -> fabric::tool::ToolCachePolicy {
        // Artifact IDs are SHA-256 content identities. Principal + Session
        // scoping prevents one caller from learning another caller's artifact
        // through the local result cache, while the digest itself supplies the
        // dependency version required for safe reuse.
        fabric::tool::ToolCachePolicy::Session { ttl_ms: 60_000 }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let reference = input
            .get("artifact_ref")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let id = reference
            .strip_prefix("artifact://sha256/")
            .unwrap_or(reference);
        let offset = input
            .get("offset")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let limit = input
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_LIMIT)
            .min(MAX_LIMIT);
        let store = self.store();
        match store.read(id) {
            Ok(content) => {
                let start_index = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(content.len());
                let end = start_index
                    .saturating_add(limit as usize)
                    .min(content.len());
                let page = String::from_utf8_lossy(&content[start_index..end]);
                ToolResult {
                    content: json!({
                        "artifact_ref": format!("artifact://sha256/{id}"),
                        "sha256": id,
                        "offset": start_index,
                        "next_offset": end,
                        "total_bytes": content.len(),
                        "complete": end == content.len(),
                        "content": page,
                    })
                    .to_string(),
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: end != content.len(),
                        patch_delta: None,
                    },
                }
            }
            Err(error) => ToolResult {
                content: format!("artifact retrieval failed: {error}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn context(root: &std::path::Path) -> ToolContext {
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: root.to_path_buf(),
            session_id: "artifact-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn reads_stable_artifact_reference_in_bounded_pages() {
        let temp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(temp.path().to_path_buf());
        let artifact = store.store(b"abcdefgh", "text/plain").unwrap();
        let tool = ArtifactReadTool {
            root: Some(temp.path().to_path_buf()),
        };
        assert_eq!(
            tool.cache_policy(),
            fabric::tool::ToolCachePolicy::Session { ttl_ms: 60_000 }
        );

        let result = tool
            .execute(
                json!({"artifact_ref": artifact.uri(), "offset": 2, "limit": 3}),
                &context(temp.path()),
            )
            .await;

        assert!(!result.is_error);
        assert!(result.metadata.truncated);
        let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["content"], "cde");
        assert_eq!(payload["next_offset"], 5);
        assert_eq!(payload["complete"], false);
    }
}
