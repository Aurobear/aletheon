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
//!     ├── Query → ParamRegistry / SubsystemRegistry / SnapshotBuilder (new)
//!     └── Stream → DebugHandler (existing)
//! ```
//!
//! ## Implementation Phases
//!
//! - **Phase A (done)**: ParamRegistry + session.param.get/list
//! - **Phase B (done)**: SnapshotBuilder + session.snapshot
//! - **Phase C (next)**: SubsystemQuery impls + session.memory/self/dasein/state
//! - **Phase D (next)**: Stream unification → delegate to DebugHandler
//! - **Phase E (next)**: session.ask + session.journal
//!
//! Design doc: `docs/plans/2026-07-03-session-gateway-design.md`

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use super::param_registry::ParamRegistry;
use super::snapshot::SnapshotBuilder;
use super::subsystem_query::SubsystemRegistry;

use crate::core::react_loop::circuit_breaker::CircuitBreakerStatus;
use crate::core::react_loop::goal_tracker::GoalTracker;
use crate::core::config::RuntimeConfig;
use crate::r#impl::daemon::debug_handler::DebugHandler;
use crate::r#impl::daemon::session_manager::SessionManager;

/// Session Gateway — unified facade for external debug access.
///
/// Routes `session.*` JSON-RPC methods to the appropriate handler.
pub struct SessionGateway {
    /// Dynamic parameter registry (Phase A).
    param_registry: Arc<ParamRegistry>,

    /// Subsystem query registry (Phase C).
    subsystem_registry: Arc<Mutex<SubsystemRegistry>>,

    /// Existing debug handler (reused for stream methods — Phase D).
    debug_handler: Arc<DebugHandler>,

    /// Snapshot data sources (Phase B).
    session_id: String,
    state: Arc<Mutex<SessionStateRef>>,
    session_manager: Arc<Mutex<SessionManager>>,
    started_at: Instant,
    runtime_config: RuntimeConfig,
}

/// Lightweight reference to SessionState internals for snapshot queries.
/// Avoids circular dependency between session_gateway and handler modules.
pub struct SessionStateRef {
    pub iteration: usize,
    pub plan_mode: bool,
    pub consecutive_errors: usize,
    pub circuit_breaker_status: CircuitBreakerStatus,
    pub tool_budget_remaining: usize,
    pub tool_budget_max: usize,
    pub recent_tools: Vec<String>,
    pub storm_breaker_failure_count: usize,
    pub goal_tracker: GoalTracker,
}

impl SessionGateway {
    /// Create a new SessionGateway.
    pub fn new(
        param_registry: Arc<ParamRegistry>,
        debug_handler: Arc<DebugHandler>,
        session_id: String,
        state: Arc<Mutex<SessionStateRef>>,
        session_manager: Arc<Mutex<SessionManager>>,
        started_at: Instant,
        runtime_config: RuntimeConfig,
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

    /// Update the snapshot state ref with current ReActLoop state.
    /// Called by the handler after each turn to keep snapshot data fresh.
    pub async fn update_state(&self, new_state: SessionStateRef) {
        let mut guard = self.state.lock().await;
        *guard = new_state;
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
            // ── Phase A: Param methods ─────────────────────────────
            "session.param.get" => Some(self.handle_param_get(id, params).await),
            "session.param.list" => Some(self.handle_param_list(id, params).await),

            // ── Phase B: Snapshot ──────────────────────────────────
            "session.snapshot" => Some(self.handle_snapshot(id).await),

            // ── Phase C: Subsystem queries (stubbed) ───────────────
            "session.memory" => Some(self.handle_memory(id, params).await),
            "session.self" => Some(self.handle_self(id, params).await),
            "session.dasein" => Some(self.handle_dasein(id).await),
            "session.state" => Some(self.handle_state(id).await),

            // ── Phase D: Stream methods (delegate to DebugHandler) ──
            "session.watch" => {
                self.debug_handler.handle_method("debug.subscribe", id, params).await
            }
            "session.topic.list" => {
                self.debug_handler.handle_method("debug.topics", id, params).await
            }
            "session.topic.echo" => {
                self.debug_handler.handle_method("debug.subscribe", id, params).await
            }
            "session.bag.record" => {
                self.debug_handler.handle_method("debug.bag_start", id, params).await
            }
            "session.bag.stop" => {
                self.debug_handler.handle_method("debug.bag_stop", id, params).await
            }
            "session.bag.play" => {
                self.debug_handler.handle_method("debug.bag_replay", id, params).await
            }
            "session.perf" => {
                self.debug_handler.handle_method("debug.perf", id, params).await
            }
            "session.log" => {
                self.debug_handler.handle_method("debug.log_subscribe", id, params).await
            }
            "session.graph" => {
                self.debug_handler.handle_method("debug.graph", id, params).await
            }

            // ── Phase E: Ask + Journal (stubbed) ──────────────────
            "session.ask" => Some(self.handle_ask(id, params).await),
            "session.journal" => Some(self.handle_journal(id, params).await),

            _ => {
                tracing::debug!(method = method, "Unknown session.* method");
                None
            }
        }
    }

    // ── Phase A: Param methods ───────────────────────────────────────────

    async fn handle_param_get(&self, id: &Value, params: &Value) -> Value {
        let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");

        match self.param_registry.get(key).await {
            Some(value) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "key": key, "value": value }
            }),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32050, "message": format!("Unknown parameter: {}", key) }
            }),
        }
    }

    async fn handle_param_list(&self, id: &Value, params: &Value) -> Value {
        let namespace = params.get("namespace").and_then(|v| v.as_str());
        let values = self.param_registry.list(namespace).await;

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "params": values }
        })
    }

    // ── Phase B: Snapshot ────────────────────────────────────────────────

    async fn handle_snapshot(&self, id: &Value) -> Value {
        let state = self.state.lock().await;
        let messages = self.session_manager.lock().await;
        let perf = self.debug_handler.perf_counter();

        let markdown = SnapshotBuilder::build(
            &self.session_id,
            &state.goal_tracker,
            perf,
            &self.runtime_config,
            self.started_at,
            state.circuit_breaker_status.clone(),
            state.tool_budget_remaining,
            state.tool_budget_max,
            &state.recent_tools,
            state.consecutive_errors,
            state.iteration,
            state.plan_mode,
            messages.message_count(),
            state.storm_breaker_failure_count,
        );

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "session_id": self.session_id,
                "snapshot": markdown,
            }
        })
    }

    // ── Phase C: Subsystem queries (stubs) ───────────────────────────────

    async fn handle_memory(&self, id: &Value, params: &Value) -> Value {
        let memory_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("all");
        let reg = self.subsystem_registry.lock().await;
        let exact_id = format!("memory.{}", memory_type);
        match reg.query(&exact_id, params) {
            Ok(markdown) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "memory_type": memory_type, "content": markdown }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": e.code, "message": e.message }
            }),
        }
    }

    async fn handle_self(&self, id: &Value, params: &Value) -> Value {
        let reg = self.subsystem_registry.lock().await;
        match reg.query("self", params) {
            Ok(markdown) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": markdown }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": e.code, "message": e.message }
            }),
        }
    }

    async fn handle_dasein(&self, id: &Value) -> Value {
        let reg = self.subsystem_registry.lock().await;
        match reg.query("dasein", &Value::Null) {
            Ok(markdown) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": markdown }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": e.code, "message": e.message }
            }),
        }
    }

    async fn handle_state(&self, id: &Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32052, "message": "session.state not yet implemented (Phase C)" }
        })
    }

    // ── Phase E: Ask + Journal (stubs) ──────────────────────────────────

    async fn handle_ask(&self, id: &Value, _params: &Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32052, "message": "session.ask not yet implemented (Phase E)" }
        })
    }

    async fn handle_journal(&self, id: &Value, _params: &Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32052, "message": "session.journal not yet implemented (Phase E)" }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};

    async fn make_gateway() -> SessionGateway {
        let param_registry = Arc::new(ParamRegistry::new());
        let debug_hook = Arc::new(tokio::sync::Mutex::new(DebugBusHook::new(
            EventFilter::default(),
        )));
        let debug_handler = Arc::new(DebugHandler::new(debug_hook, Arc::new(PerfCounter::default())));
        let state = Arc::new(Mutex::new(SessionStateRef {
            iteration: 0,
            plan_mode: false,
            consecutive_errors: 0,
            circuit_breaker_status: CircuitBreakerStatus::Ok,
            tool_budget_remaining: 10,
            tool_budget_max: 10,
            recent_tools: vec![],
            storm_breaker_failure_count: 0,
            goal_tracker: GoalTracker::new(),
        }));
        // SessionManager — use temp dir, create asynchronously
        let tmp = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(tmp.path(), "test-session".into(), 100000)
            .await
            .unwrap();

        SessionGateway::new(
            param_registry,
            debug_handler,
            "test-session".into(),
            state,
            Arc::new(Mutex::new(sm)),
            Instant::now(),
            RuntimeConfig::default(),
        )
    }

    #[tokio::test]
    async fn param_get_and_list() {
        let gw = make_gateway().await;
        gw.param_registry
            .declare("test.x", "test", "x param", || json!(42))
            .await;

        let resp = gw
            .handle_method(
                "session.param.get",
                &json!("1"),
                &json!({"key": "test.x"}),
            )
            .await
            .unwrap();
        assert_eq!(resp["result"]["key"], "test.x");
        assert_eq!(resp["result"]["value"], json!(42));
    }

    #[tokio::test]
    async fn snapshot_returns_markdown() {
        let gw = make_gateway().await;
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
        let gw = make_gateway().await;
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
        let gw = make_gateway().await;
        assert!(gw
            .handle_method("chat", &json!("1"), &Value::Null)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn stub_methods_return_not_implemented() {
        let gw = make_gateway().await;
        for method in &["session.state", "session.ask", "session.journal"] {
            let resp = gw
                .handle_method(method, &json!("1"), &Value::Null)
                .await
                .unwrap();
            assert!(resp.get("error").is_some(), "{} should return error", method);
        }
    }

    #[tokio::test]
    async fn stream_methods_delegate() {
        let gw = make_gateway().await;

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
}
