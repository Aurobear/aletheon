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
use crate::CoreMemory;
use crate::RecallMemory;
use dasein::SelfField;

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

    /// Memory and SelfField refs (Phase C).
    core_memory: Arc<Mutex<CoreMemory>>,
    recall_memory: Arc<Mutex<RecallMemory>>,
    self_field: Arc<Mutex<SelfField>>,
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
        core_memory: Arc<Mutex<CoreMemory>>,
        recall_memory: Arc<Mutex<RecallMemory>>,
        self_field: Arc<Mutex<SelfField>>,
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

    // ── Phase C: Subsystem queries ─────────────────────────────────────

    async fn handle_memory(&self, id: &Value, params: &Value) -> Value {
        let memory_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("all");

        let mut md = String::from("# Memory\n\n");

        if memory_type == "all" || memory_type == "core" {
            let cm = self.core_memory.lock().await;
            md.push_str("## Core Memory Blocks\n\n");
            for (label, block) in cm.blocks() {
                md.push_str(&format!(
                    "### {}\n- char_limit: {}\n- read_only: {}\n\n{}\n\n",
                    label, block.char_limit, block.read_only, block.value
                ));
            }
        }

        if memory_type == "all" || memory_type == "recall" {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let rm = self.recall_memory.lock().await;
            md.push_str("## Recall Memory (Recent)\n\n");
            match rm.recent(limit) {
                Ok(entries) if !entries.is_empty() => {
                    for entry in &entries {
                        md.push_str(&format!(
                            "- **[{}]** {}\n",
                            entry.entry_type, entry.content
                        ));
                    }
                }
                _ => {
                    md.push_str("*(no entries)*\n");
                }
            }
            md.push('\n');
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "memory_type": memory_type, "content": md }
        })
    }

    async fn handle_self(&self, id: &Value, params: &Value) -> Value {
        let layer = params.get("layer").and_then(|v| v.as_str()).unwrap_or("all");
        let sf = self.self_field.lock().await;

        let mut md = String::from("# SelfField State\n\n");

        if layer == "all" || layer == "identity" {
            md.push_str("## Identity\n");
            use base::Subsystem;
            md.push_str(&format!("- Name: {}\n", sf.name()));
            md.push_str(&format!("- Version: {}\n\n", sf.version()));
        }

        if layer == "all" || layer == "boundary" {
            md.push_str("## Boundary\n");
            let _boundary = sf.boundary();
            md.push_str("- Boundary layer active\n\n");
        }

        if layer == "all" || layer == "dasein" {
            md.push_str("## DaseinModule\n");
            if let Some(ref d) = sf.dasein() {
                let m = d.mood();
                md.push_str(&format!("- Mood: {:?}\n", m));
                md.push_str(&format!("- Sorge alive: {}\n", d.is_alive()));

                let ts = d.temporality();
                let tss = ts.to_snapshot();
                md.push_str(&format!("- Retention depth: {}\n", tss.recent_retentions.len()));
                md.push_str(&format!("- Tempo: {:.2}\n", tss.tempo));

                let w = d.world();
                md.push_str(&format!("- Bewandtnis nodes: {} nodes, {} edges\n",
                    w.node_count(), w.edge_count()));

                let sm = d.self_model();
                md.push_str(&format!("- Self-assertions: {}\n", sm.assertion_count()));

                let cs = d.care();
                let css = cs.to_snapshot();
                md.push_str(&format!("- Concerns: {}\n", css.concerns.len()));
                md.push_str(&format!("- Rhythm interval: {}ms\n", css.rhythm_interval_ms));
            } else {
                md.push_str("*(DaseinModule not enabled)*\n");
            }
            md.push('\n');
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "layer": layer, "content": md }
        })
    }

    async fn handle_dasein(&self, id: &Value) -> Value {
        let sf = self.self_field.lock().await;
        let mut md = String::from("# DaseinModule State\n\n");

        if let Some(ref d) = sf.dasein() {
            md.push_str("## Stimmung (Mood)\n");
            md.push_str(&format!("- {:?}\n\n", d.mood()));

            md.push_str("## TemporalStream\n");
            let tss = d.temporality().to_snapshot();
            md.push_str(&format!("- Recent retentions: {}\n", tss.recent_retentions.len()));
            md.push_str(&format!(
                "- Present: action={:?}, mood_tone={:?}\n",
                tss.present.action, tss.present.mood_tone
            ));
            md.push_str(&format!("- Protentions: {}\n", tss.protentions.len()));
            md.push_str(&format!("- Tempo: {:.2}\n\n", tss.tempo));

            md.push_str("## Bewandtnisganzheit (World)\n");
            let ws = d.world().to_snapshot();
            md.push_str(&format!(
                "- Ready-to-hand: {} | Present-at-hand: {} | Unavailable: {}\n",
                ws.ready_to_hand.len(), ws.present_at_hand.len(), ws.unavailable.len()
            ));
            md.push_str(&format!("- Ultimate concern: {:?}\n\n", ws.ultimate_concern));

            md.push_str("## MutableSelfModel\n");
            let sms = d.self_model().to_snapshot();
            md.push_str(&format!("- Current assertions: {}\n", sms.current_assertions.len()));
            for a in &sms.current_assertions {
                md.push_str(&format!("  - \"{}\" (stability: {:.2})\n", a.content, a.stability));
            }
            md.push_str(&format!("- Negated assertions: {}\n", sms.negated_assertions.len()));
            md.push_str(&format!("- Possibilities: {}\n\n", sms.possibilities.len()));

            md.push_str("## CareStructure\n");
            let css = d.care().to_snapshot();
            md.push_str(&format!("- Projection: {:?}\n", css.projection));
            md.push_str(&format!("- Concerns: {}\n", css.concerns.len()));
            for c in &css.concerns {
                md.push_str(&format!(
                    "  - \"{}\" (urgency: {:.2})\n", c.purpose, c.urgency
                ));
            }
            md.push_str(&format!("- Rhythm interval: {}ms\n", css.rhythm_interval_ms));
            md.push_str(&format!("- Fallenness depth: {:.2}\n\n", css.fallenness_depth));

            md.push_str("## SorgeLoop\n");
            md.push_str(&format!("- Alive: {}\n", d.is_alive()));
        } else {
            md.push_str("*(DaseinModule not enabled)*\n");
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": md }
        })
    }

    async fn handle_state(&self, id: &Value) -> Value {
        let state = self.state.lock().await;
        let messages = self.session_manager.lock().await;

        let mut md = String::from("# ReActLoop State\n\n");

        md.push_str("## Loop State\n");
        md.push_str(&format!("- Iteration: {}\n", state.iteration));
        md.push_str(&format!(
            "- Max iterations: {}\n",
            self.runtime_config.max_iterations
        ));
        md.push_str(&format!(
            "- Plan mode: {}\n",
            if state.plan_mode { "yes" } else { "no" }
        ));
        md.push_str(&format!("- Consecutive errors: {}\n", state.consecutive_errors));

        md.push_str("\n## Tool Budget\n");
        md.push_str(&format!(
            "- Used: {} / {}\n",
            state.tool_budget_max - state.tool_budget_remaining,
            state.tool_budget_max
        ));
        if !state.recent_tools.is_empty() {
            md.push_str("- Recent tools:\n");
            for t in state.recent_tools.iter().rev().take(10) {
                md.push_str(&format!("  - {}\n", t));
            }
        }

        md.push_str("\n## Circuit Breaker\n");
        md.push_str(&format!("- Status: {:?}\n", state.circuit_breaker_status));

        md.push_str("\n## Goal Tracker\n");
        md.push_str(&state.goal_tracker.get_context());
        md.push('\n');

        md.push_str("## Session\n");
        md.push_str(&format!("- Messages: {}\n", messages.message_count()));
        md.push_str(&format!(
            "- Estimated tokens: {}\n\n",
            messages.estimate_tokens()
        ));

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": md }
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
        let tmp = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(tmp.path(), "test-session".into(), 100000)
            .await
            .unwrap();

        // Create test CoreMemory, RecallMemory, SelfField
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db = tmp.path().join("recall.db");
        let recall_memory = Arc::new(Mutex::new(
            RecallMemory::new(&recall_db).unwrap(),
        ));
        let self_field = Arc::new(Mutex::new(SelfField::new(Default::default())));

        SessionGateway::new(
            param_registry,
            debug_handler,
            "test-session".into(),
            state,
            Arc::new(Mutex::new(sm)),
            Instant::now(),
            RuntimeConfig::default(),
            core_memory,
            recall_memory,
            self_field,
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
        // Only ask + journal are still stubbed
        for method in &["session.ask", "session.journal"] {
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

    #[tokio::test]
    async fn memory_query_returns_blocks() {
        let gw = make_gateway().await;
        let resp = gw
            .handle_method("session.memory", &json!("1"), &json!({"type": "core"}))
            .await
            .unwrap();
        assert!(resp["result"]["content"].as_str().unwrap().contains("Core Memory"));
    }

    #[tokio::test]
    async fn self_query_returns_identity() {
        let gw = make_gateway().await;
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
        let gw = make_gateway().await;
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
        let gw = make_gateway().await;
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
