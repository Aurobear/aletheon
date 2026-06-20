//! Debug handler — exposes debug API via JSON-RPC.
//!
//! Implements `debug.*` methods called by `aletheon debug` CLI subcommands.
//!
//! Design: `docs/plans/2026-06-19-aletheon-debug-system-design.md` (Layer 3).

use aletheon_abi::debug::{DebugEvent, DebugLevel};
use aletheon_comm::r#impl::debug_bus::{
    DebugBusHook, EventFilter, EventRecorder, PerfCounter,
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
    id: String,
    path: PathBuf,
    recorder: EventRecorder,
    started_at: Instant,
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
}

impl DebugHandler {
    pub fn new(hook: Arc<Mutex<DebugBusHook>>, perf: Arc<PerfCounter>) -> Self {
        Self {
            hook,
            perf,
            subscribers: Mutex::new(HashMap::new()),
            recordings: Mutex::new(HashMap::new()),
            started_at: Instant::now(),
        }
    }

    /// Handle a debug.* JSON-RPC method.
    ///
    /// Returns `None` if the method is not a debug method (caller should handle it).
    /// Returns `Some(response)` for debug methods.
    pub async fn handle_method(&self, method: &str, id: &Value, params: &Value) -> Option<Value> {
        match method {
            "debug.topics" => Some(self.handle_topics(id).await),
            "debug.subscribe" => Some(self.handle_subscribe(id, params).await),
            "debug.unsubscribe" => Some(self.handle_unsubscribe(id, params).await),
            "debug.node_info" => Some(self.handle_node_info(id).await),
            "debug.bag_start" => Some(self.handle_bag_start(id, params).await),
            "debug.bag_stop" => Some(self.handle_bag_stop(id, params).await),
            "debug.bag_replay" => Some(self.handle_bag_replay(id, params).await),
            "debug.perf" => Some(self.handle_perf(id).await),
            "debug.trace_start" => Some(self.handle_trace_start(id, params).await),
            "debug.trace_stop" => Some(self.handle_trace_stop(id).await),
            "debug.trace_status" => Some(self.handle_trace_status(id).await),
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

    async fn handle_subscribe(&self, id: &Value, params: &Value) -> Value {
        let filter = parse_event_filter(params);
        let (tx, _rx) = mpsc::channel::<DebugEvent>(256);
        let sub_id = uuid::Uuid::new_v4().to_string();

        self.subscribers.lock().await.insert(sub_id.clone(), tx);

        // Update the hook's filter so matching events are forwarded
        self.hook.lock().await.set_filter(filter);

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "subscribed": true,
                "subscription_id": sub_id,
                "message": "Use 'aletheon debug topic echo' to stream events"
            }
        })
    }

    async fn handle_unsubscribe(&self, id: &Value, params: &Value) -> Value {
        let sub_id = params.get("id").and_then(|v| v.as_str()).unwrap_or("");
        self.subscribers.lock().await.remove(sub_id);

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

        let recorder = EventRecorder::new(path.clone(), max_buffer);
        let rec_id = uuid::Uuid::new_v4().to_string();

        self.recordings.lock().await.insert(
            rec_id.clone(),
            ActiveRecording {
                id: rec_id.clone(),
                path: path.clone(),
                recorder,
                started_at: Instant::now(),
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
            Some(rec) => match rec.recorder.stop().await {
                Ok(meta) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "stopped": true,
                        "path": meta.path.to_string_lossy(),
                        "events": meta.event_count,
                        "duration_secs": meta.duration.as_secs_f64(),
                    }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32040, "message": format!("Failed to stop recording: {}", e) }
                }),
            },
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

        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => {
                let events: Vec<Value> = contents
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "replayed": true,
                        "events": events.len(),
                        "speed": speed,
                        "path": path.to_string_lossy(),
                    }
                })
            }
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32043, "message": format!("Failed to read bag file: {}", e) }
            }),
        }
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
        self.hook.lock().await.set_filter(EventFilter::default());

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tracing": false }
        })
    }

    async fn handle_trace_status(&self, id: &Value) -> Value {
        // We don't store the current filter separately, so report "off"
        // unless there are active subscribers.
        let sub_count = self.subscribers.lock().await.len();
        let tracing = sub_count > 0;

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tracing": tracing }
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
