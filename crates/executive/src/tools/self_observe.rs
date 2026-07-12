//! self_observe tool — lets the LLM observe its own Dasein state.
//!
//! Queries: mood, temporality, world, self_model, care, full.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use fabric::dasein::DaseinOps;
use fabric::tool::{
    ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult, ToolResultMeta,
};

/// Tool that exposes Dasein internal state to the LLM.
///
/// Generic over any `DaseinOps` implementation so it works with
/// the real `DaseinModule` or a mock.
pub struct SelfObserveTool<T: DaseinOps> {
    dasein: Arc<T>,
}

impl<T: DaseinOps> SelfObserveTool<T> {
    pub fn new(dasein: Arc<T>) -> Self {
        Self { dasein }
    }
}

#[async_trait]
impl<T: DaseinOps + 'static> Tool for SelfObserveTool<T> {
    fn name(&self) -> &str {
        "self_observe"
    }

    fn description(&self) -> &str {
        "Observe your own internal state: mood, experiences, world, self-model, care."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "enum": ["mood", "temporality", "world", "self_model", "care", "full"],
                    "description": "What to observe"
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        // Cannot clone a generic T easily; this tool is intended to be
        // constructed once and wrapped in Arc<dyn Tool> at registration time.
        panic!(
            "SelfObserveTool does not support boxed_clone; wrap in Arc<dyn Tool> at registration"
        )
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("full");
        let ctx = self.dasein.to_context_injection();
        let start = std::time::Instant::now();

        let content = match query {
            "mood" => format!("Mood: {:?}", ctx.mood),
            "temporality" => format!(
                "Retentions: {}, Protentions: {}",
                ctx.temporality.recent_retentions.len(),
                ctx.temporality.protentions.len()
            ),
            "world" => format!(
                "Ready: {}, PresentAtHand: {}, Unavailable: {}",
                ctx.world.ready_to_hand.len(),
                ctx.world.present_at_hand.len(),
                ctx.world.unavailable.len()
            ),
            "self_model" => format!(
                "Assertions: {}, Negated: {}, Possibilities: {}",
                ctx.self_model.current_assertions.len(),
                ctx.self_model.negated_assertions.len(),
                ctx.self_model.possibilities.len()
            ),
            "care" => format!(
                "Concerns: {}, Fallenness: {:.2}, Rhythm: {}ms",
                ctx.care.concerns.len(),
                ctx.care.fallenness_depth,
                ctx.care.rhythm_interval_ms
            ),
            "full" => format!("{:#?}", ctx),
            other => format!(
                "Unknown query: {}. Valid: mood, temporality, world, self_model, care, full",
                other
            ),
        };

        ToolResult {
            content,
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: start.elapsed().as_millis() as u64,
                truncated: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::dasein::*;

    /// Minimal mock DaseinOps for testing.
    struct MockDasein;

    #[async_trait]
    impl DaseinOps for MockDasein {
        fn mood(&self) -> Stimmung {
            Stimmung::Gelassenheit
        }
        fn temporality_snapshot(&self) -> TemporalStreamSnapshot {
            TemporalStreamSnapshot {
                recent_retentions: vec![],
                present: PresentSnapshot {
                    semantic: "test".into(),
                    action: None,
                    perception: None,
                    mood_tone: Stimmung::Gelassenheit,
                },
                protentions: vec![],
                tempo: 1.0,
            }
        }
        fn world_snapshot(&self) -> BewandtnisSnapshot {
            BewandtnisSnapshot {
                ready_to_hand: vec![],
                present_at_hand: vec![],
                unavailable: vec![],
                ultimate_concern: None,
            }
        }
        fn self_model_snapshot(&self) -> SelfModelSnapshot {
            SelfModelSnapshot {
                current_assertions: vec![],
                negated_assertions: vec![],
                possibilities: vec![],
            }
        }
        fn care_snapshot(&self) -> CareStructureSnapshot {
            CareStructureSnapshot {
                projection: None,
                constraints: vec![],
                absorbed_in: None,
                fallenness_depth: 0.0,
                concerns: vec![],
                rhythm_interval_ms: 1000,
            }
        }
        fn to_context_injection(&self) -> DaseinContext {
            DaseinContext {
                mood: self.mood(),
                temporality: self.temporality_snapshot(),
                world: self.world_snapshot(),
                self_model: self.self_model_snapshot(),
                care: self.care_snapshot(),
            }
        }
        async fn handle_event(&self, _event: DaseinEvent) -> anyhow::Result<()> {
            Ok(())
        }
        async fn start_sorge_loop(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn stop_sorge_loop(&self) -> anyhow::Result<()> {
            Ok(())
        }
        fn is_alive(&self) -> bool {
            true
        }
    }

    fn make_tool() -> SelfObserveTool<MockDasein> {
        SelfObserveTool::new(Arc::new(MockDasein))
    }

    #[tokio::test]
    async fn test_mood_query() {
        let tool = make_tool();
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool.execute(json!({"query": "mood"}), &ctx).await;
        assert!(result.content.contains("Gelassenheit"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_full_query() {
        let tool = make_tool();
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool.execute(json!({"query": "full"}), &ctx).await;
        assert!(result.content.contains("DaseinContext"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_unknown_query() {
        let tool = make_tool();
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool.execute(json!({"query": "bogus"}), &ctx).await;
        assert!(result.content.contains("Unknown query"));
    }

    #[tokio::test]
    async fn test_default_query_is_full() {
        let tool = make_tool();
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.content.contains("DaseinContext"));
    }

    #[test]
    fn test_name_and_schema() {
        let tool = make_tool();
        assert_eq!(tool.name(), "self_observe");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        let schema = tool.input_schema();
        assert!(schema["properties"]["query"]["enum"].is_array());
    }
}
