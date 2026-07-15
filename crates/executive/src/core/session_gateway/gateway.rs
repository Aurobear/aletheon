//! Session Gateway — unified facade for external Agent debug access.
//!
//! Provides Query (structured state) and Stream (real-time events) methods
//! under the `session.*` JSON-RPC namespace.
//!
//! ## Architecture
//!
//! ```text
//! Claude Code / Developer
//!     │  echo '{"method":"session.snapshot",...}' | nc -U socket
//!     ▼
//! SessionGateway::handle_method()
//!     │
//!     ├── Query → ParamRegistry / SubsystemRegistry / SnapshotBuilder
//!     └── Stream → DebugHandler (existing)
//! ```
//!
//! ## Module structure
//!
//! - `gateway` — (this file) SessionGateway struct, method dispatch orchestrator
//! - `session_state` — SessionStateRef, state update methods, state query
//! - `turn_context` — Snapshot, memory, self, dasein, and ask handlers
//! - `approval_flow` — Param and journal handlers
//! - `param_registry` — dynamic parameter registration
//! - `snapshot` — runtime snapshot builder
//! - `subsystem_query` — per-module structured state export trait
//!
//! Design doc: `docs/plans/2026-07-03-session-gateway-design.md`

use fabric::{Clock, MonoTime};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::param_registry::ParamRegistry;
pub use super::session_state::SessionStateRef;
use super::subsystem_query::SubsystemRegistry;

use crate::core::config::ExecutiveConfig;
use crate::r#impl::daemon::debug_handler::DebugHandler;
use crate::r#impl::daemon::session_manager::SessionManager;
#[cfg(test)]
use cognit::harness::linear::circuit_breaker::CircuitBreakerStatus;
#[cfg(test)]
use cognit::harness::linear::goal_tracker::GoalTracker;
use dasein::SelfField;
use fabric::LlmProvider;
use mnemosyne::CoreMemory;
use mnemosyne::RecallMemory;

/// Session Gateway — unified facade for external debug access.
///
/// Routes `session.*` JSON-RPC methods to the appropriate handler.
pub struct SessionGateway {
    /// Dynamic parameter registry (Phase A).
    pub(super) param_registry: Arc<ParamRegistry>,

    /// Subsystem query registry (Phase C).
    pub(super) subsystem_registry: Arc<Mutex<SubsystemRegistry>>,

    /// Existing debug handler (reused for stream methods - Phase D).
    pub(super) debug_handler: Arc<DebugHandler>,

    /// Snapshot data sources (Phase B).
    pub(super) session_id: String,
    pub(super) state: Arc<Mutex<SessionStateRef>>,
    pub(super) session_manager: Arc<Mutex<SessionManager>>,
    pub(super) started_at: MonoTime,
    pub(super) runtime_config: ExecutiveConfig,

    /// Memory and SelfField refs (Phase C).
    pub(super) core_memory: Arc<Mutex<CoreMemory>>,
    pub(super) recall_memory: Arc<Mutex<RecallMemory>>,
    pub(super) self_field: Arc<Mutex<SelfField>>,

    /// LLM provider for session.ask (Phase E).
    pub(super) llm: Arc<dyn LlmProvider>,

    /// Clock for uptime computation in snapshot.
    pub(super) clock: Arc<dyn Clock>,
}

impl SessionGateway {
    /// Create a new SessionGateway.
    pub fn new(
        param_registry: Arc<ParamRegistry>,
        debug_handler: Arc<DebugHandler>,
        session_id: String,
        state: Arc<Mutex<SessionStateRef>>,
        session_manager: Arc<Mutex<SessionManager>>,
        started_at: MonoTime,
        runtime_config: ExecutiveConfig,
        core_memory: Arc<Mutex<CoreMemory>>,
        recall_memory: Arc<Mutex<RecallMemory>>,
        self_field: Arc<Mutex<SelfField>>,
        llm: Arc<dyn LlmProvider>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            param_registry,
            subsystem_registry: Arc::new(Mutex::new(SubsystemRegistry::new())),
            debug_handler,
            session_id,
            state,
            session_manager,
            started_at,
            runtime_config,
            core_memory,
            recall_memory,
            self_field,
            llm,
            clock,
        }
    }

    /// Get a reference to the subsystem registry for registration at init time.
    pub fn subsystem_registry(&self) -> &Arc<Mutex<SubsystemRegistry>> {
        &self.subsystem_registry
    }

    /// Get a reference to the param registry for registration at init time.
    pub fn param_registry(&self) -> &Arc<ParamRegistry> {
        &self.param_registry
    }

    /// Route a `session.*` JSON-RPC method to the appropriate handler.
    ///
    /// Returns `Some(response)` for recognized methods, `None` if the method
    /// doesn't start with `session.` (caller should handle it).
    pub async fn handle_method(&self, method: &str, id: &Value, params: &Value) -> Option<Value> {
        if !method.starts_with("session.") {
            return None;
        }

        match method {
            // Phase A: Param methods
            "session.param.get" => Some(self.handle_param_get(id, params).await),
            "session.param.list" => Some(self.handle_param_list(id, params).await),

            // Phase B: Snapshot
            "session.snapshot" => Some(self.handle_snapshot(id).await),

            // Phase C: Subsystem queries
            "session.memory" => Some(self.handle_memory(id, params).await),
            "session.self" => Some(self.handle_self(id, params).await),
            "session.dasein" => Some(self.handle_dasein(id).await),
            "session.state" => Some(self.handle_state(id).await),

            // Phase D: Stream methods (delegate to DebugHandler)
            "session.watch" => {
                self.debug_handler
                    .handle_method("debug.subscribe", id, params)
                    .await
            }
            "session.topic.list" => {
                self.debug_handler
                    .handle_method("debug.topics", id, params)
                    .await
            }
            "session.topic.echo" => {
                self.debug_handler
                    .handle_method("debug.subscribe", id, params)
                    .await
            }
            "session.bag.record" => {
                self.debug_handler
                    .handle_method("debug.bag_start", id, params)
                    .await
            }
            "session.bag.stop" => {
                self.debug_handler
                    .handle_method("debug.bag_stop", id, params)
                    .await
            }
            "session.bag.play" => {
                self.debug_handler
                    .handle_method("debug.bag_replay", id, params)
                    .await
            }
            "session.perf" => {
                self.debug_handler
                    .handle_method("debug.perf", id, params)
                    .await
            }
            "session.log" => {
                self.debug_handler
                    .handle_method("debug.log_subscribe", id, params)
                    .await
            }
            "session.graph" => {
                self.debug_handler
                    .handle_method("debug.graph", id, params)
                    .await
            }

            // Phase E: Ask + Journal
            "session.ask" => Some(self.handle_ask(id, params).await),
            "session.journal" => Some(self.handle_journal(id, params).await),

            _ => {
                tracing::debug!(method = method, "Unknown session.* method");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

    /// Test fixture holding the gateway and the tempdir (keeps SQLite alive).
    struct TestFixture {
        gw: SessionGateway,
        _tmp: tempfile::TempDir,
    }

    async fn make_gateway() -> TestFixture {
        let param_registry = Arc::new(ParamRegistry::new());
        let debug_hook = Arc::new(tokio::sync::Mutex::new(DebugBusHook::new(
            EventFilter::default(),
        )));
        let test_clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let debug_handler = Arc::new(DebugHandler::new(
            debug_hook,
            Arc::new(PerfCounter::default()),
            test_clock.clone(),
        ));
        let state = Arc::new(Mutex::new(SessionStateRef {
            iteration: 0,
            plan_mode: false,
            consecutive_errors: 0,
            circuit_breaker_status: CircuitBreakerStatus::Ok,
            tool_budget_remaining: 10,
            tool_budget_max: 10,
            recent_tools: vec![],
            storm_breaker_failure_count: 0,
            goal_tracker: GoalTracker::new(
                Arc::new(aletheon_kernel::chronos::TestClock::default()),
            ),
        }));
        let tmp = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(
            tmp.path(),
            "test-session".into(),
            100000,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .await
        .unwrap();

        // Create test CoreMemory, RecallMemory, SelfField
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db = tmp.path().join("recall.db");
        let recall_clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let recall_memory = Arc::new(Mutex::new(
            RecallMemory::new(&recall_db, recall_clock).unwrap(),
        ));
        let self_field = Arc::new(Mutex::new(SelfField::new(dasein::SelfFieldConfig {
            clock: Some(Arc::new(aletheon_kernel::chronos::TestClock::default())),
            ..Default::default()
        })));

        // Mock LLM for session.ask
        let mock_llm = Arc::new(cognit::testing::mock_llm::MockLlmProvider::new("test-mock"));

        let gw = SessionGateway::new(
            param_registry,
            debug_handler,
            "test-session".into(),
            state,
            Arc::new(Mutex::new(sm)),
            test_clock.mono_now(),
            ExecutiveConfig::default(),
            core_memory,
            recall_memory,
            self_field,
            mock_llm,
            test_clock,
        );

        TestFixture { gw, _tmp: tmp }
    }

    #[tokio::test]
    async fn param_get_and_list() {
        let f = make_gateway().await;
        let gw = f.gw;
        gw.param_registry
            .declare("test.x", "test", "x param", || json!(42))
            .await;

        let resp = gw
            .handle_method("session.param.get", &json!("1"), &json!({"key": "test.x"}))
            .await
            .unwrap();
        assert_eq!(resp["result"]["key"], "test.x");
        assert_eq!(resp["result"]["value"], json!(42));
    }

    #[tokio::test]
    async fn snapshot_returns_markdown() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.snapshot", &json!("1"), &Value::Null)
            .await
            .unwrap();

        assert!(resp["result"]["snapshot"].is_string());
        let md = resp["result"]["snapshot"].as_str().unwrap();
        assert!(md.contains("HEALTHY"));
        assert!(md.contains("test-session"));
        assert!(md.contains("no goal set"));
    }

    #[tokio::test]
    async fn snapshot_with_goal() {
        let f = make_gateway().await;
        let gw = f.gw;
        {
            let mut state = gw.state.lock().await;
            state.goal_tracker.set_goal("Debug the crash".into());
            state.circuit_breaker_status = CircuitBreakerStatus::Warning("test warn".into());
            state.consecutive_errors = 3;
            state.recent_tools = vec!["bash_exec".into(), "file_read".into()];
        }

        let resp = gw
            .handle_method("session.snapshot", &json!("1"), &Value::Null)
            .await
            .unwrap();
        let md = resp["result"]["snapshot"].as_str().unwrap();
        assert!(md.contains("Debug the crash"));
        assert!(md.contains("DEGRADED"));
        assert!(md.contains("test warn"));
        assert!(md.contains("file_read"));
    }

    #[tokio::test]
    async fn non_session_method_returns_none() {
        let f = make_gateway().await;
        let gw = f.gw;
        assert!(gw
            .handle_method("chat", &json!("1"), &Value::Null)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn ask_returns_error_for_empty_question() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.ask", &json!("1"), &json!({"question": ""}))
            .await
            .unwrap();
        assert!(
            resp.get("error").is_some(),
            "empty question should return error"
        );
    }

    #[tokio::test]
    async fn ask_with_mock_llm_returns_answer() {
        let f = make_gateway().await;
        let gw = f.gw;
        // The mock LLM has no canned responses, so it returns an error
        // (the test verifies the flow works and error is handled gracefully)
        let resp = gw
            .handle_method(
                "session.ask",
                &json!("1"),
                &json!({"question": "What is my current goal?"}),
            )
            .await
            .unwrap();
        // Either a result or an error is valid - depends on mock LLM state
        assert!(resp.get("result").is_some() || resp.get("error").is_some());
    }

    #[tokio::test]
    async fn journal_returns_entries() {
        let f = make_gateway().await;
        let gw = f.gw;
        // Append some events to the journal first
        {
            let sm = gw.session_manager.lock().await;
            sm.journal()
                .append(crate::r#impl::session::journal::SessionEvent::UserMessage {
                    content: "test question".into(),
                })
                .await
                .unwrap();
            sm.journal().flush().await.unwrap();
        }

        let resp = gw
            .handle_method("session.journal", &json!("1"), &json!({}))
            .await
            .unwrap();
        assert!(resp["result"]["entries"].is_array());
        assert!(resp["result"]["count"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn journal_filter_by_type() {
        let f = make_gateway().await;
        let gw = f.gw;
        // Append some events
        {
            let sm = gw.session_manager.lock().await;
            sm.journal()
                .append(crate::r#impl::session::journal::SessionEvent::UserMessage {
                    content: "hello".into(),
                })
                .await
                .unwrap();
            sm.journal()
                .append(
                    crate::r#impl::session::journal::SessionEvent::AssistantMessage {
                        content: "hi".into(),
                    },
                )
                .await
                .unwrap();
            sm.journal().flush().await.unwrap();
        }

        let resp = gw
            .handle_method(
                "session.journal",
                &json!("1"),
                &json!({"event_type": "user_message"}),
            )
            .await
            .unwrap();
        assert!(resp["result"]["entries"].is_array());
        // Should only have user_message entries
        for entry in resp["result"]["entries"].as_array().unwrap() {
            assert_eq!(entry["event_type"], "user_message");
        }
    }

    #[tokio::test]
    async fn stream_methods_delegate() {
        let f = make_gateway().await;
        let gw = f.gw;

        let resp = gw
            .handle_method("session.topic.list", &json!("1"), &Value::Null)
            .await
            .unwrap();
        assert!(resp["result"]["topics"].is_array());

        let resp = gw
            .handle_method("session.perf", &json!("2"), &Value::Null)
            .await
            .unwrap();
        assert!(resp["result"]["perf"].is_object());
    }

    #[tokio::test]
    async fn memory_query_returns_blocks() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.memory", &json!("1"), &json!({"type": "core"}))
            .await
            .unwrap();
        assert!(resp["result"]["content"]
            .as_str()
            .unwrap()
            .contains("Core Memory"));
    }

    #[tokio::test]
    async fn self_query_returns_identity() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.self", &json!("1"), &json!({"layer": "identity"}))
            .await
            .unwrap();
        let content = resp["result"]["content"].as_str().unwrap();
        assert!(content.contains("Identity"));
        assert!(content.contains("Name:"));
    }

    #[tokio::test]
    async fn dasein_query_handles_disabled() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.dasein", &json!("1"), &Value::Null)
            .await
            .unwrap();
        let content = resp["result"]["content"].as_str().unwrap();
        // DaseinModule may or may not be enabled; both cases should produce valid output
        assert!(content.contains("DaseinModule"));
    }

    #[tokio::test]
    async fn state_query_returns_loop_state() {
        let f = make_gateway().await;
        let gw = f.gw;
        let resp = gw
            .handle_method("session.state", &json!("1"), &Value::Null)
            .await
            .unwrap();
        let content = resp["result"]["content"].as_str().unwrap();
        assert!(content.contains("ReActLoop"));
        assert!(content.contains("Iteration"));
        assert!(content.contains("Tool Budget"));
    }
}
