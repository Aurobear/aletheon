//! Debug handler — exposes debug API via JSON-RPC.
//!
//! Implements `debug.*` methods called by `aletheon debug` CLI subcommands.
//!
//! Design: `docs/plans/2026-06-19-aletheon-debug-system-design.md` (Layer 3).

use base::kernel::debug::{DebugEvent, DebugLevel};
use base::kernel::debug_bus::{
    DebugBusHook, EventFilter, EventRecorder, PerfCounter, RecorderSink, SubscriberSink,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

// ---------------------------------------------------------------------------
// Tracepoint catalog
// ---------------------------------------------------------------------------

/// Static catalog of known tracepoints.
fn builtin_tracepoints() -> Vec<Value> {
    vec![
        json!({"name": "react_loop.iteration", "module": "runtime", "level": "debug", "description": "ReAct loop iteration started"}),
        json!({"name": "tool.dispatch", "module": "body", "level": "info", "description": "Tool call dispatched"}),
        json!({"name": "tool.result", "module": "body", "level": "info", "description": "Tool call result returned"}),
        json!({"name": "memory.stored", "module": "memory", "level": "info", "description": "Fact stored to memory"}),
        json!({"name": "memory.recall", "module": "memory", "level": "info", "description": "Memory recall triggered"}),
        json!({"name": "turn.start", "module": "runtime", "level": "info", "description": "Chat turn started"}),
        json!({"name": "turn.end", "module": "runtime", "level": "info", "description": "Chat turn completed"}),
        json!({"name": "llm.request", "module": "brain", "level": "debug", "description": "LLM API request sent"}),
        json!({"name": "llm.response", "module": "brain", "level": "debug", "description": "LLM API response received"}),
        json!({"name": "selffield.review", "module": "self", "level": "debug", "description": "SelfField intent review"}),
        json!({"name": "hook.execute", "module": "runtime", "level": "debug", "description": "Lifecycle hook executed"}),
    ]
}

// ---------------------------------------------------------------------------
// Active recording
// ---------------------------------------------------------------------------

struct ActiveRecording {
    /// Recording session identifier — reserved for future correlation.
    #[allow(dead_code)]
    id: String,
    path: PathBuf,
    started_at: Instant,
    /// Index of the RecorderSink in the DebugBusHook's sinks vec.
    sink_index: usize,
}

// ---------------------------------------------------------------------------
// DebugHandler
// ---------------------------------------------------------------------------

/// Debug handler — manages debug state and processes debug.* JSON-RPC methods.
pub struct DebugHandler {
    hook: Arc<Mutex<DebugBusHook>>,
    perf: Arc<PerfCounter>,
    subscribers: Mutex<HashMap<String, mpsc::Sender<DebugEvent>>>,
    recordings: Mutex<HashMap<String, ActiveRecording>>,
    started_at: Instant,
    /// Pending subscriber receivers, waiting for the server to drain them.
    /// Populated by `subscribe()`, consumed by `take_pending_subscriber_rx()`.
    pending_subscriber_rx: Mutex<Option<mpsc::Receiver<DebugEvent>>>,
}

impl DebugHandler {
    pub fn new(hook: Arc<Mutex<DebugBusHook>>, perf: Arc<PerfCounter>) -> Self {
        Self {
            hook,
            perf,
            subscribers: Mutex::new(HashMap::new()),
            recordings: Mutex::new(HashMap::new()),
            started_at: Instant::now(),
            pending_subscriber_rx: Mutex::new(None),
        }
    }

    /// Take the pending subscriber receiver (if a subscribe was just processed).
    /// Returns `Some(rx)` if `debug.subscribe` was just called, `None` otherwise.
    pub async fn take_pending_subscriber_rx(&self) -> Option<mpsc::Receiver<DebugEvent>> {
        self.pending_subscriber_rx.lock().await.take()
    }

    /// Get a reference to the performance counter (shared with SessionGateway).
    pub fn perf_counter(&self) -> &PerfCounter {
        &self.perf
    }

    /// Handle a debug.* JSON-RPC method.
    ///
    /// Returns `None` if the method is not a debug method (caller should handle it).
    /// Returns `Some(response)` for debug methods.
    ///
    /// For `debug.subscribe`, the subscriber receiver is stored internally and
    /// can be taken via `take_pending_subscriber_rx()`.
    pub async fn handle_method(&self, method: &str, id: &Value, params: &Value) -> Option<Value> {
        match method {
            "debug.topics" => Some(self.handle_topics(id).await),
            "debug.subscribe" => {
                let (resp, rx) = self.subscribe(id, params).await;
                *self.pending_subscriber_rx.lock().await = Some(rx);
                Some(resp)
            }
            "debug.unsubscribe" => Some(self.handle_unsubscribe(id, params).await),
            "debug.node_info" => Some(self.handle_node_info(id).await),
            "debug.bag_start" => Some(self.handle_bag_start(id, params).await),
            "debug.bag_stop" => Some(self.handle_bag_stop(id, params).await),
            "debug.bag_replay" => Some(self.handle_bag_replay(id, params).await),
            "debug.perf" => Some(self.handle_perf(id).await),
            "debug.trace_start" => Some(self.handle_trace_start(id, params).await),
            "debug.trace_stop" => Some(self.handle_trace_stop(id).await),
            "debug.trace_status" => Some(self.handle_trace_status(id).await),
            "debug.health" => Some(self.handle_health(id).await),
            "debug.nodes" => Some(self.handle_nodes(id).await),
            "debug.param_get" => Some(self.handle_param_get(id, params).await),
            "debug.param_list" => Some(self.handle_param_list(id).await),
            "debug.graph" => Some(self.handle_graph(id).await),
            "debug.topology" => Some(self.handle_topology(id).await),
            "debug.log_subscribe" => Some(self.handle_log_subscribe(id, params).await),
            _ => None,
        }
    }

    // ── topics ───────────────────────────────────────────────────────────────

    async fn handle_topics(&self, id: &Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "topics": builtin_tracepoints() }
        })
    }

    // ── subscribe / unsubscribe ──────────────────────────────────────────────

    /// Subscribe to debug events. Returns (response_json, subscriber_receiver).
    /// The caller is responsible for draining the receiver to the client socket.
    pub async fn subscribe(&self, id: &Value, params: &Value) -> (Value, mpsc::Receiver<DebugEvent>) {
        let filter = parse_event_filter(params);
        let (tx, rx) = mpsc::channel::<DebugEvent>(256);
        let sub_id = uuid::Uuid::new_v4().to_string();

        self.subscribers.lock().await.insert(sub_id.clone(), tx.clone());

        // Register a SubscriberSink with its OWN filter (per-sink filtering)
        let sink = Arc::new(SubscriberSink::new(sub_id.clone(), tx, filter));
        self.hook.lock().await.add_sink(sink);

        // DO NOT update the hook's global filter — each sink has its own

        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "subscribed": true,
                "subscription_id": sub_id,
                "message": "Use 'aletheon debug topic echo' to stream events"
            }
        });
        (response, rx)
    }

    async fn handle_unsubscribe(&self, id: &Value, params: &Value) -> Value {
        let sub_id = params.get("id").and_then(|v| v.as_str()).unwrap_or("");
        self.subscribers.lock().await.remove(sub_id);
        self.hook.lock().await.remove_sink_by_id(sub_id);

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "ok": true }
        })
    }

    // ── node_info ────────────────────────────────────────────────────────────

    async fn handle_node_info(&self, id: &Value) -> Value {
        let perf = self.perf.snapshot();
        let uptime = self.started_at.elapsed();
        let rss_kb = read_rss_kb().unwrap_or(0);

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "node_info": {
                    "pid": std::process::id(),
                    "uptime_secs": uptime.as_secs(),
                    "uptime_human": format_duration(uptime),
                    "memory_rss_kb": rss_kb,
                    "tokens_in": perf.tokens_in,
                    "tokens_out": perf.tokens_out,
                    "turn_count": perf.turn_count,
                    "error_count": perf.error_count,
                }
            }
        })
    }

    // ── bag record ───────────────────────────────────────────────────────────

    async fn handle_bag_start(&self, id: &Value, params: &Value) -> Value {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                PathBuf::from(format!("/tmp/aletheon/bag_{}.jsonl", ts))
            });

        let max_buffer = params
            .get("max_buffer")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000) as usize;

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let rec_id = uuid::Uuid::new_v4().to_string();

        // Register a RecorderSink on the DebugBusHook so events flow into the recorder
        let recorder = EventRecorder::new(path.clone(), max_buffer);
        let sink = Arc::new(RecorderSink::new(recorder));
        let sink_index = self.hook.lock().await.add_sink(sink);

        // Track the recording for stop/metadata
        self.recordings.lock().await.insert(
            rec_id.clone(),
            ActiveRecording {
                id: rec_id.clone(),
                path: path.clone(),
                started_at: Instant::now(),
                sink_index,
            },
        );

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "recording_id": rec_id,
                "path": path.to_string_lossy(),
            }
        })
    }

    async fn handle_bag_stop(&self, id: &Value, params: &Value) -> Value {
        let rec_id = params
            .get("recording_id")
            .or_else(|| params.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let recording = self.recordings.lock().await.remove(&rec_id);

        match recording {
            Some(rec) => {
                // Remove the RecorderSink from the hook and get the recorded events
                let duration = rec.started_at.elapsed();
                let mut hook = self.hook.lock().await;
                hook.remove_sink(rec.sink_index);

                // The RecorderSink was removed; we report basic metadata.
                // The sink's EventRecorder was flushed via the sink's Drop or
                // we can't access it after remove. Report path and duration.
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "stopped": true,
                        "path": rec.path.to_string_lossy(),
                        "duration_secs": duration.as_secs_f64(),
                        "message": "Recording stopped. Events written to bag file."
                    }
                })
            }
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32041, "message": format!("No active recording with id: {}", rec_id) }
            }),
        }
    }

    // ── bag replay ───────────────────────────────────────────────────────────

    async fn handle_bag_replay(&self, id: &Value, params: &Value) -> Value {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => PathBuf::from(p),
            None => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32042, "message": "Missing 'path' parameter" }
                });
            }
        };

        let speed = params
            .get("speed")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);

        let contents = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32043, "message": format!("Failed to read bag file: {}", e) }
                });
            }
        };

        // Parse all DebugEvent entries from the bag file
        let events: Vec<DebugEvent> = contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        let event_count = events.len();
        let hook = self.hook.clone();
        let _perf = self.perf.clone();

        // Spawn async replay task — re-publish events with timing
        tokio::spawn(async move {
            let mut prev_ts: u64 = 0;
            for event in events {
                // Respect original timing scaled by speed
                if speed > 0.0 && prev_ts > 0 && event.ts > prev_ts {
                    let delay_ms = (event.ts - prev_ts) as f64 / speed;
                    if delay_ms > 0.0 && delay_ms < 60_000.0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
                    }
                }
                prev_ts = event.ts;

                // Re-publish through the hook so subscribers receive it
                let mut h = hook.lock().await;
                h.on_event(&event).await;
            }
        });

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "replaying": true,
                "events": event_count,
                "speed": speed,
                "path": path.to_string_lossy(),
            }
        })
    }

    // ── perf ─────────────────────────────────────────────────────────────────

    async fn handle_perf(&self, id: &Value) -> Value {
        let snap = self.perf.snapshot();
        let tool_calls = {
            let map = self.perf.tool_calls.lock().await;
            map.clone()
        };
        let tool_calls_json: Value = serde_json::to_value(&tool_calls).unwrap_or_default();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "perf": {
                    "tokens_in": snap.tokens_in,
                    "tokens_out": snap.tokens_out,
                    "tokens_total": snap.tokens_in + snap.tokens_out,
                    "turn_count": snap.turn_count,
                    "error_count": snap.error_count,
                    "tool_calls": tool_calls_json,
                }
            }
        })
    }

    // ── trace ────────────────────────────────────────────────────────────────

    async fn handle_trace_start(&self, id: &Value, params: &Value) -> Value {
        let module = params.get("module").and_then(|v| v.as_str());
        let level = params
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("debug");

        let min_level = parse_level(level);

        let mut modules = None;
        if let Some(m) = module {
            let mut s = HashSet::new();
            s.insert(m.to_string());
            modules = Some(s);
        }

        self.hook.lock().await.set_filter(EventFilter {
            min_level,
            modules,
            tracepoints: None,
        });

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tracing": true,
                "level": level,
                "module": module,
            }
        })
    }

    async fn handle_trace_stop(&self, id: &Value) -> Value {
        // Clear subscriber sinks, keep recorder sinks
        self.hook.lock().await.clear_subscriber_sinks();
        // Clear subscriber map
        self.subscribers.lock().await.clear();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tracing": false }
        })
    }

    async fn handle_trace_status(&self, id: &Value) -> Value {
        let sub_count = self.subscribers.lock().await.len();
        let tracing = sub_count > 0;
        let hook = self.hook.lock().await;
        let filter = hook.current_filter();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tracing": tracing,
                "subscribers": sub_count,
                "level": format!("{:?}", filter.min_level).to_lowercase(),
                "modules": filter.modules,
            }
        })
    }

    // ── health ───────────────────────────────────────────────────────────────

    async fn handle_health(&self, id: &Value) -> Value {
        let perf = self.perf.snapshot();
        let tool_calls = {
            let map = self.perf.tool_calls.lock().await;
            map.clone()
        };
        let uptime = self.started_at.elapsed();
        let rss_kb = read_rss_kb().unwrap_or(0);
        let sub_count = self.subscribers.lock().await.len();
        let rec_count = self.recordings.lock().await.len();

        // Determine overall status
        let overall = if perf.error_count == 0 { "HEALTHY" } else { "DEGRADED" };

        let mut warnings = Vec::new();
        if perf.error_count > 0 {
            warnings.push(format!("{} errors recorded", perf.error_count));
        }
        if rss_kb > 500_000 {
            warnings.push(format!("High memory usage: {} MB", rss_kb / 1024));
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "health": {
                    "overall": overall,
                    "pid": std::process::id(),
                    "uptime_secs": uptime.as_secs(),
                    "uptime_human": format_duration(uptime),
                    "memory_rss_mb": rss_kb / 1024,
                    "tokens_in": perf.tokens_in,
                    "tokens_out": perf.tokens_out,
                    "turn_count": perf.turn_count,
                    "error_count": perf.error_count,
                    "tool_calls": serde_json::to_value(&tool_calls).unwrap_or_default(),
                    "active_subscribers": sub_count,
                    "active_recordings": rec_count,
                    "warnings": warnings,
                }
            }
        })
    }

    // ── nodes ────────────────────────────────────────────────────────────────

    async fn handle_nodes(&self, id: &Value) -> Value {
        let perf = self.perf.snapshot();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "nodes": [
                    {
                        "name": "daemon",
                        "running": true,
                        "status_line": format!("uptime={}, turns={}", format_duration(self.started_at.elapsed()), perf.turn_count),
                        "details": {
                            "pid": std::process::id(),
                            "tokens_in": perf.tokens_in,
                            "tokens_out": perf.tokens_out,
                            "error_count": perf.error_count,
                        }
                    }
                ]
            }
        })
    }

    // ── param ────────────────────────────────────────────────────────────────

    async fn handle_param_get(&self, id: &Value, params: &Value) -> Value {
        let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");

        // Return known configuration values
        let value = match key {
            "agent.max_iterations" => json!(25),
            "agent.max_tokens" => json!(100000),
            "agent.default_provider" => json!("deepseek"),
            "agent.default_model" => json!("deepseek-v4-flash"),
            "debug.subscriber_count" => {
                let count = self.subscribers.lock().await.len();
                json!(count)
            }
            "debug.recording_count" => {
                let count = self.recordings.lock().await.len();
                json!(count)
            }
            "debug.uptime_secs" => json!(self.started_at.elapsed().as_secs()),
            "debug.memory_rss_kb" => json!(read_rss_kb().unwrap_or(0)),
            _ => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32050, "message": format!("Unknown parameter: {}", key) }
                });
            }
        };

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "key": key, "value": value }
        })
    }

    async fn handle_param_list(&self, id: &Value) -> Value {
        let sub_count = self.subscribers.lock().await.len();
        let rec_count = self.recordings.lock().await.len();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "params": {
                    "agent.max_iterations": 25,
                    "agent.max_tokens": 100000,
                    "agent.default_provider": "deepseek",
                    "agent.default_model": "deepseek-v4-flash",
                    "debug.subscriber_count": sub_count,
                    "debug.recording_count": rec_count,
                    "debug.uptime_secs": self.started_at.elapsed().as_secs(),
                    "debug.memory_rss_kb": read_rss_kb().unwrap_or(0),
                }
            }
        })
    }

    // ── graph ────────────────────────────────────────────────────────────────

    async fn handle_graph(&self, id: &Value) -> Value {
        // Returns the event flow topology as a directed graph
        // Nodes = subsystems, Edges = event flow direction
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "graph": {
                    "nodes": [
                        {"id": "user",        "label": "User Input",     "type": "io"},
                        {"id": "tui",         "label": "TUI",            "type": "io"},
                        {"id": "server",      "label": "UnixServer",     "type": "network"},
                        {"id": "handler",     "label": "RequestHandler", "type": "core"},
                        {"id": "react_loop",  "label": "ReActLoop",      "type": "core"},
                        {"id": "llm",         "label": "LLM Provider",   "type": "external"},
                        {"id": "tools",       "label": "ToolRunner",     "type": "core"},
                        {"id": "event_sink",  "label": "EventSink",      "type": "core"},
                        {"id": "debug_bus",   "label": "DebugBusHook",   "type": "debug"},
                        {"id": "session_mgr", "label": "SessionManager", "type": "core"},
                        {"id": "fact_store",  "label": "FactStore",      "type": "memory"},
                        {"id": "hooks",       "label": "HookRegistry",   "type": "core"},
                        {"id": "self_field",  "label": "SelfField",      "type": "core"},
                        {"id": "perception",  "label": "Perception",     "type": "core"},
                    ],
                    "edges": [
                        {"from": "user",       "to": "tui",         "label": "keyboard"},
                        {"from": "tui",        "to": "server",      "label": "json-rpc"},
                        {"from": "server",     "to": "handler",     "label": "request"},
                        {"from": "handler",    "to": "react_loop",  "label": "chat"},
                        {"from": "handler",    "to": "hooks",       "label": "pre/post turn"},
                        {"from": "handler",    "to": "self_field",  "label": "review"},
                        {"from": "handler",    "to": "fact_store",  "label": "recall"},
                        {"from": "handler",    "to": "session_mgr", "label": "push/compact"},
                        {"from": "react_loop", "to": "llm",         "label": "completion"},
                        {"from": "react_loop", "to": "tools",       "label": "tool_call"},
                        {"from": "react_loop", "to": "event_sink",  "label": "events"},
                        {"from": "event_sink", "to": "tui",         "label": "notify"},
                        {"from": "event_sink", "to": "debug_bus",   "label": "debug events"},
                        {"from": "debug_bus",  "to": "subscribers", "label": "debug subscribe"},
                        {"from": "perception", "to": "handler",     "label": "inject"},
                    ]
                }
            }
        })
    }

    // ── topology (DOT format) ────────────────────────────────────────────────

    async fn handle_topology(&self, id: &Value) -> Value {
        let dot = r#"digraph aletheon {
    rankdir=LR;
    node [shape=box, style=filled, fillcolor=lightblue];

    user [label="User", shape=ellipse, fillcolor=lightyellow];
    tui [label="TUI"];
    server [label="UnixServer"];
    handler [label="RequestHandler"];
    react_loop [label="ReActLoop"];
    llm [label="LLM Provider", fillcolor=lightpink];
    tools [label="ToolRunner"];
    event_sink [label="EventSink"];
    debug_bus [label="DebugBusHook", fillcolor=lightgreen];
    session_mgr [label="SessionManager"];
    fact_store [label="FactStore", fillcolor=wheat];
    hooks [label="HookRegistry"];
    self_field [label="SelfField"];
    perception [label="Perception"];

    user -> tui [label="input"];
    tui -> server [label="json-rpc"];
    server -> handler [label="dispatch"];
    handler -> react_loop [label="chat"];
    handler -> hooks [label="pre/post"];
    handler -> self_field [label="review"];
    handler -> fact_store [label="recall"];
    handler -> session_mgr [label="state"];
    react_loop -> llm [label="completion"];
    react_loop -> tools [label="tool_call"];
    react_loop -> event_sink [label="events"];
    event_sink -> tui [label="notify"];
    event_sink -> debug_bus [label="debug"];
    debug_bus -> subscribers [label="subscribe", style=dashed];
    perception -> handler [label="inject", style=dashed];
}"#;

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "topology": {
                    "format": "dot",
                    "dot": dot,
                }
            }
        })
    }

    // ── log_subscribe (structured log streaming) ─────────────────────────────

    async fn handle_log_subscribe(&self, id: &Value, params: &Value) -> Value {
        let level = params
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info");
        let module_filter = params.get("module").and_then(|v| v.as_str());

        let min_level = parse_level(level);

        // Create a subscriber that captures log-level events
        let filter = EventFilter {
            min_level,
            modules: module_filter.map(|m| HashSet::from([m.to_string()])),
            tracepoints: None,
        };

        let (tx, rx) = mpsc::channel::<DebugEvent>(512);
        let sub_id = uuid::Uuid::new_v4().to_string();

        self.subscribers.lock().await.insert(sub_id.clone(), tx.clone());
        let sink = Arc::new(SubscriberSink::new(sub_id.clone(), tx, filter));
        self.hook.lock().await.add_sink(sink);

        *self.pending_subscriber_rx.lock().await = Some(rx);

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "subscribed": true,
                "subscription_id": sub_id,
                "level": level,
                "module": module_filter,
                "message": "Log stream started. Events will be forwarded as notifications."
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_event_filter(params: &Value) -> EventFilter {
    let min_level = params
        .get("level")
        .and_then(|v| v.as_str())
        .map(parse_level)
        .unwrap_or(DebugLevel::Off);

    let modules = params
        .get("module")
        .and_then(|v| v.as_str())
        .map(|m| HashSet::from([m.to_string()]));

    let tracepoints = params
        .get("tracepoint")
        .and_then(|v| v.as_str())
        .map(|t| HashSet::from([t.to_string()]));

    EventFilter {
        min_level,
        modules,
        tracepoints,
    }
}

fn parse_level(s: &str) -> DebugLevel {
    match s.to_lowercase().as_str() {
        "error" => DebugLevel::Error,
        "warn" | "warning" => DebugLevel::Warn,
        "info" => DebugLevel::Info,
        "debug" => DebugLevel::Debug,
        "trace" => DebugLevel::Trace,
        _ => DebugLevel::Info,
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn read_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].parse().ok();
            }
        }
    }
    None
}
