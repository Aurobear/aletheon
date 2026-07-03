# Session Gateway Design

> Version: v1.0
>
> Status: Design Proposal
>
> Purpose: Define the Session Gateway — a unified facade that exposes all Aletheon
> runtime state for external debug agents (Claude Code, developers) to inspect,
> query, and interact with a running Agent session.

---

## 1. Motivation

### 1.1 The debug.loop.md Vision

`docs/guide/debug.loop.md` proposes a self-debug loop:

```
Aletheon Running → Detect anomaly → Context Port (inspect) → Chat Port (ask)
→ External Agent (Claude/Codex) analysis → Patch proposal → Human review → Merge
```

The key insight: **Aletheon exposes problems; external reasoning models analyze them.**

### 1.2 Current State vs Target

| debug.loop.md concept | Current state | Gap |
|---|---|---|
| Session Gateway (unified entry) | 3 separate handlers (chat/clear/status + debug.* + TuiSessionManager) | No unified API |
| Context Port (inspect runtime) | debug.node_info (minimal), debug.subscribe (events only) | No structured snapshot |
| Chat Port (ask questions to agent) | chat method (LLM direct, no runtime context) | No context-aware ask |
| Watch Port (event stream) | debug.subscribe (fully implemented) | Needs tracepoint expansion |
| Runtime Snapshot | Data scattered across 39 RequestHandler fields + 6 memory systems + 8 SelfField layers | No aggregation |
| External Agent attach | None | Needs protocol-level access |

### 1.3 The Opportunity

Aletheon already has a ROS-style debug infrastructure:

| ROS concept | Aletheon equivalent | Status |
|---|---|---|
| `rostopic list` | `debug.topics` (11 builtin tracepoints) | Done |
| `rostopic echo` | `debug.subscribe` (streaming) | Done |
| `rosbag record/play` | `debug.bag_start/stop/replay` | Done |
| `rostopic hz` | `debug hz` | Done |
| `rosparam get/list` | `debug.param_get/list` (8 hard-coded values) | Stub |
| `rqt_graph` | `debug.graph/topology` | Done |

The Session Gateway builds on this foundation — adding **structured state queries** on top of the existing **streaming event infrastructure**.

---

## 2. Architecture

### 2.1 Three-Layer Design

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 3: Session Gateway (session.* namespace)  ← NEW      │
│                                                             │
│  Query (structured state)         Stream (real-time events) │
│  ─────────────────────            ──────────────────────    │
│  session.snapshot()               session.watch(filter)     │
│  session.memory(type)             session.topic.echo(tp)    │
│  session.self()                   session.topic.list()      │
│  session.dasein()                 session.bag.record/play   │
│  session.state()                  session.perf              │
│  session.param.get/list           session.log(...)          │
│  session.ask(msg)                                           │
│  session.journal(from,n)                                    │
├────────────────────┬────────────────────────────────────────┤
│  Layer 2: Infra    │                                        │
│  (new + reuse)     │                                        │
│                    │                                        │
│  ParamRegistry (N) │  DebugBusHook (R)  EventRecorder (R)   │
│  SnapshotBuilder(N)│  PerfCounter  (R)  tracepoints   (R)   │
│  SubsystemQuery(N) │                                        │
├────────────────────┴────────────────────────────────────────┤
│  Layer 1: Module State (unchanged)                          │
│                                                             │
│  RequestHandler(39 fields)  SessionManager  EventJournal    │
│  CoreMemory  RecallMemory  FactStore  EpisodicMemory        │
│  SelfField(8 layers)  DaseinModule(7 subsystems)            │
│  ReActLoop(19 fields)  GoalTracker  StormBreaker            │
│  Perception  Evolution  MetricsExporter  ToolTracker        │
└─────────────────────────────────────────────────────────────┘

N = New, R = Reused
```

### 2.2 Core Principle

**Session Gateway is a read-only facade.** It does not change how subsystems work internally. It only:

1. Provides a unified JSON-RPC namespace (`session.*`)
2. Routes Query methods to new infrastructure (ParamRegistry, SnapshotBuilder, SubsystemQuery)
3. Routes Stream methods to existing infrastructure (DebugBusHook, EventRecorder, PerfCounter)
4. Formats all Query responses as **markdown** (human + Claude readable, compact)

Existing `debug.*` methods remain for backward compatibility but no new ones are added.

---

## 3. Session Gateway API

All methods use JSON-RPC 2.0 over the Unix socket at `/run/aletheond/aletheond.sock`,
same as the existing daemon protocol.

### 3.1 session.snapshot()

Returns a one-page runtime summary as markdown.

```json
{
  "method": "session.snapshot",
  "params": {}
}
```

Response:

```markdown
# Aletheon Runtime Snapshot — session: abc123

## Current Goal
- investigate disk usage anomaly in /var/log

## Plan
1. du -sh /var/log/* | sort -rh
2. find files > 100M
3. identify log rotation issue

## Mode
auto

## Health
- Status: HEALTHY
- Uptime: 14m 23s
- Iteration: 8/50
- Consecutive errors: 0

## Recent Events
- [tool] bash_exec completed (1.2s, ok) — "du -sh /var/log/*"
- [tool] file_read completed (0.3s, ok) — "/var/log/syslog (first 200 lines)"
- [memory] CoreMemory.learned appended — "Disk usage pattern..."
- [reflection] OnTrack — "Making steady progress..."

## Resource Usage
- RSS: 42.3 MB
- Tokens used: 14,230 / 100,000
- Tool calls this turn: 3 / 10

## Open Errors
(none)

## Active Configuration
- Model: deepseek-v4-flash
- Provider: deepseek
- Sandbox: auto
```

**Implementation:** `SnapshotBuilder` aggregates from:
- `GoalTracker::current_goal()`, `GoalTracker::sub_goals()`
- `ReActLoop::iteration`, `ReActLoop::messages` (last 5 events)
- `PerfCounter` (tokens, errors, turns)
- `CircuitBreaker` status, `ToolBudget::remaining()`
- `read_rss_kb()`, `started_at.elapsed()`
- `RuntimeConfig`, `DaemonConfig`

### 3.2 session.memory(type)

Query a specific memory subsystem.

```json
{
  "method": "session.memory",
  "params": {
    "type": "core",
    "limit": 20
  }
}
```

| `type` value | Returns | Source |
|---|---|---|
| `"core"` | All core memory blocks with values | `CoreMemory` |
| `"recall"` | Recent recall entries (by recency) | `RecallMemory` (SQLite FTS5) |
| `"facts"` | Top facts by trust score | `FactStore` (SQLite) |
| `"episodic"` | Recent episodic events | `EpisodicMemory` (SQLite) |
| `"all"` | Summary of all memory systems | Aggregated counts + samples |

Response: Markdown, one section per block/entry.

### 3.3 session.self()

Query SelfField state.

```json
{
  "method": "session.self",
  "params": {
    "layer": "all"
  }
}
```

| `layer` value | Returns |
|---|---|
| `"all"` | All 8 layers + DaseinModule summary |
| `"boundary"` | Boundary rules, recent verdicts |
| `"identity"` | Current identity + mutation history |
| `"care"` | Care topics with weights |
| `"narrative"` | Recent narrative entries |
| `"attention"` | Current attention topics |
| `"dasein"` | DaseinModule full state (see `session.dasein()`) |

### 3.4 session.dasein()

Query DaseinModule existential state.

```json
{
  "method": "session.dasein",
  "params": {}
}
```

Returns markdown covering:
- **Stimmung**: Current mood, intensity, trend
- **TemporalStream**: Retention depth, present urimpression, protention field
- **Bewandtnisganzheit**: Entity graph size, ultimate concern
- **MutableSelfModel**: Current assertions, negated history
- **CareStructure**: Projection, thrownness, concerns
- **SorgeLoop**: Running status, event queue depth

### 3.5 session.state()

Query ReActLoop internal state.

```json
{
  "method": "session.state",
  "params": {}
}
```

Returns markdown covering:
- `iteration`, `max_iterations`
- `plan_mode`, `consecutive_errors`
- `tool_budget` (used/max, recent tools)
- `circuit_breaker` (tripped? recent calls)
- `reflection_engine` (interval, last classification)
- `goal_tracker` (current goal, sub-goals, success criteria)
- Message buffer size, compaction count

### 3.6 session.param.get / session.param.list

Dynamic parameter query system. Replaces the 8 hard-coded values in `debug.param_get/list`.

```json
{
  "method": "session.param.get",
  "params": {
    "key": "react.tool_calls_remaining"
  }
}
```

```json
{
  "method": "session.param.list",
  "params": {
    "namespace": "react"
  }
}
```

### 3.7 session.ask()

Ask a question directly to the running Agent, in the context of the current session.

```json
{
  "method": "session.ask",
  "params": {
    "message": "Why did you call bash_exec with 'rm -rf /tmp/*'?"
  }
}
```

The message is injected into the session as a system-level query (not a user message).
The Agent responds with its reasoning context and recent tool history available.

**Design note:** `session.ask` is synchronous: it constructs a lightweight LLM query
with the current session messages as context plus the ask message as a system prompt suffix.
It does NOT go through the full ReActLoop (no tool execution). It does NOT bypass
`SelfField.review()` — the ask is reviewed as a `SessionQuery` intent before LLM call.
The response is the raw LLM output (text only, no tool calls).

### 3.8 session.journal()

Query event journal history.

```json
{
  "method": "session.journal",
  "params": {
    "from": 0,
    "limit": 50,
    "type": "tool_call"
  }
}
```

| `type` filter | Matches |
|---|---|
| `"all"` | All events |
| `"user"` | UserMessage |
| `"assistant"` | AssistantMessage |
| `"tool_call"` | ToolCallStarted + ToolCallCompleted |
| `"checkpoint"` | CheckpointBoundary |
| `"error"` | ToolCallCompleted with is_error=true |

Response format: Markdown table with timestamp, event_type, and summary. For structured
consumption, add `"format": "json"` to params to get the raw JSON array.

### 3.9 Stream Methods (Routing to DebugBusHook)

These are thin wrappers over existing `debug.*` infrastructure:

| `session.*` method | Maps to | Description |
|---|---|---|
| `session.watch(filter)` | `debug.subscribe` | Real-time debug event stream |
| `session.topic.list()` | `debug.topics` | List all tracepoints |
| `session.topic.echo(tp)` | `debug.subscribe` with tracepoint filter | Stream specific tracepoint |
| `session.bag.record()` | `debug.bag_start` | Start recording |
| `session.bag.stop()` | `debug.bag_stop` | Stop recording |
| `session.bag.play(path)` | `debug.bag_replay` | Replay recording |
| `session.perf` | `debug.perf` | Performance counters |
| `session.log(filter)` | `debug.log_subscribe` | Structured log streaming |
| `session.graph()` | `debug.graph` | Event flow topology |

These methods do NOT duplicate implementation. They delegate to existing `DebugHandler` methods.

---

## 4. ParamRegistry Design

### 4.1 Problem

Current `debug.param_get/list` has 8 hard-coded values in a `match`/`json!()` block.
No module can register new params without editing `debug_handler.rs`.

### 4.2 Design

```rust
/// Dynamic parameter registry with lazy evaluation.
///
/// Each parameter is a key + a getter closure that produces a JSON value.
/// Getters are called on each query (no caching), ensuring live values.
pub struct ParamRegistry {
    params: RwLock<HashMap<String, ParamEntry>>,
}

struct ParamEntry {
    /// Human-readable description.
    description: &'static str,
    /// Namespace for grouping (e.g., "react", "self", "memory").
    namespace: &'static str,
    /// Getter: called each time the param is read.
    getter: Box<dyn Fn() -> serde_json::Value + Send + Sync>,
}

impl ParamRegistry {
    /// Register a parameter.
    ///
    /// # Example
    /// ```ignore
    /// registry.declare(
    ///     "react.tool_calls_remaining",
    ///     "react",
    ///     "Number of tool calls remaining in current turn budget",
    ///     || json!(tool_budget.remaining()),
    /// );
    /// ```
    pub fn declare(
        &self,
        key: &str,
        namespace: &str,
        description: &str,
        getter: impl Fn() -> serde_json::Value + Send + Sync + 'static,
    );

    /// Get a single parameter value.
    pub fn get(&self, key: &str) -> Option<serde_json::Value>;

    /// List all parameters, optionally filtered by namespace.
    pub fn list(&self, namespace: Option<&str>) -> HashMap<String, serde_json::Value>;

    /// Dump all parameters with their descriptions (for debugging).
    pub fn dump(&self) -> Vec<ParamInfo>;
}
```

### 4.3 Registration Points

Each subsystem registers its params at init time:

```rust
// In RequestHandler::new(), after each subsystem is initialized:

// ReActLoop params
param_registry.declare("react.iteration", "react",
    "Current ReAct loop iteration number",
    || json!(state.lock().unwrap().runtime.current_iteration()));
param_registry.declare("react.tool_calls_remaining", "react",
    "Remaining tool call budget this turn",
    || json!(state.lock().unwrap().runtime.tool_budget_remaining()));
param_registry.declare("react.max_iterations", "react",
    "Maximum iterations per turn",
    || json!(config.agent_loop.max_iterations));
param_registry.declare("react.circuit_breaker_tripped", "react",
    "Whether the circuit breaker has tripped",
    || json!(state.lock().unwrap().runtime.circuit_breaker_tripped()));
param_registry.declare("react.consecutive_errors", "react",
    "Consecutive tool execution errors",
    || json!(state.lock().unwrap().runtime.consecutive_errors()));

// SelfField params
self_field.register_params(&param_registry);

// Dasein params
if let Some(ref dasein) = self_field.dasein() {
    dasein.register_params(&param_registry);
}

// Memory params
param_registry.declare("memory.core_blocks", "memory",
    "Number of active core memory blocks",
    || json!(core_memory.lock().unwrap().block_count()));
param_registry.declare("memory.recall_entries", "memory",
    "Number of recall memory entries",
    || json!(recall_memory.lock().unwrap().entry_count()));
param_registry.declare("memory.fact_count", "memory",
    "Number of facts in fact store",
    || json!(fact_store.lock().unwrap().fact_count()));

// Session params
param_registry.declare("session.uptime_secs", "session",
    "Daemon uptime in seconds",
    || json!(started_at.elapsed().as_secs()));
param_registry.declare("session.rss_kb", "session",
    "Resident memory in KB",
    || json!(read_rss_kb().unwrap_or(0)));
param_registry.declare("session.message_count", "session",
    "Number of messages in current conversation",
    || json!(session_manager.lock().unwrap().message_count()));

// LLM params
param_registry.declare("llm.model", "llm",
    "Current LLM model in use",
    || json!(model));
param_registry.declare("llm.provider", "llm",
    "Current LLM provider name",
    || json!(provider_name));

// Perception params
param_registry.declare("perception.watch_paths", "perception",
    "Filesystem paths being watched",
    || json!(perception_config.watch_paths));

// Sandbox params
param_registry.declare("sandbox.preference", "sandbox",
    "Current sandbox mode",
    || json!(sandbox_preference));
```

### 4.4 Namespaced List Query

`session.param.list` with namespace filter:

```json
// All params
{"method": "session.param.list", "params": {}}

// Only react params
{"method": "session.param.list", "params": {"namespace": "react"}}

// Only memory params
{"method": "session.param.list", "params": {"namespace": "memory"}}
```

---

## 5. SubsystemQuery Trait

### 5.1 Design

Each subsystem implements a common query interface for structured state export:

```rust
/// Trait implemented by subsystems that can export their state for debugging.
///
/// The `query()` method returns markdown — the Session Gateway's universal format.
pub trait SubsystemQuery {
    /// Unique subsystem identifier (e.g., "memory.core", "self.boundary").
    fn subsystem_id(&self) -> &'static str;

    /// Export this subsystem's current state as markdown.
    ///
    /// `params` — optional query parameters (e.g., layer name, limit, filter).
    fn query(&self, params: &serde_json::Value) -> Result<String, QueryError>;
}

pub struct QueryError {
    pub code: i32,
    pub message: String,
}
```

### 5.2 Implementations

```rust
impl SubsystemQuery for CoreMemory {
    fn subsystem_id(&self) -> &'static str { "memory.core" }
    fn query(&self, _params: &Value) -> Result<String, QueryError> {
        let mut md = String::from("# Core Memory\n\n");
        for (label, block) in &self.blocks {
            md.push_str(&format!("## {}\n- char_limit: {}\n- read_only: {}\n\n{}\n\n",
                label, block.char_limit, block.read_only, block.value));
        }
        Ok(md)
    }
}

impl SubsystemQuery for RecallMemory {
    fn subsystem_id(&self) -> &'static str { "memory.recall" }
    fn query(&self, params: &Value) -> Result<String, QueryError> {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let entries = self.search_recent(limit);
        // Format as markdown...
        Ok(md)
    }
}

impl SubsystemQuery for SelfField {
    fn subsystem_id(&self) -> &'static str { "self" }
    fn query(&self, params: &Value) -> Result<String, QueryError> {
        let layer = params.get("layer").and_then(|v| v.as_str()).unwrap_or("all");
        // Export SelfField layers as markdown...
        Ok(md)
    }
}

impl SubsystemQuery for DaseinModule {
    fn subsystem_id(&self) -> &'static str { "dasein" }
    fn query(&self, _params: &Value) -> Result<String, QueryError> {
        // Export Stimmung, TemporalStream, Bewandtnis, SelfModel, CareStructure...
        Ok(md)
    }
}
```

### 5.3 Registry

```rust
pub struct SubsystemRegistry {
    subsystems: HashMap<&'static str, Box<dyn SubsystemQuery + Send + Sync>>,
}

impl SubsystemRegistry {
    pub fn register(&mut self, subsystem: Box<dyn SubsystemQuery + Send + Sync>) {
        self.subsystems.insert(subsystem.subsystem_id(), subsystem);
    }

    pub fn query(&self, id: &str, params: &Value) -> Result<String, QueryError> {
        match self.subsystems.get(id) {
            Some(s) => s.query(params),
            None => Err(QueryError { code: -32051, message: format!("Unknown subsystem: {}", id) }),
        }
    }

    pub fn list_subsystems(&self) -> Vec<&'static str> {
        self.subsystems.keys().copied().collect()
    }
}
```

---

## 6. SnapshotBuilder

### 6.1 Design

`SnapshotBuilder` aggregates from multiple subsystems to produce a single markdown page:

```rust
pub struct SnapshotBuilder {
    session_id: String,
    subsystems: Arc<SubsystemRegistry>,
    param_registry: Arc<ParamRegistry>,
}

impl SnapshotBuilder {
    pub async fn build(
        &self,
        goal_tracker: &GoalTracker,
        perf: &PerfCounter,
        config: &RuntimeConfig,
        started_at: Instant,
        circuit_breaker_status: CircuitBreakerStatus,
        tool_budget_status: ToolBudgetStatus,
        recent_events: Vec<String>,
    ) -> Result<String, QueryError> {
        // Compose markdown from all sources
    }
}
```

**Data sources for snapshot:**

| Section | Source | Access |
|---|---|---|
| Current Goal | `GoalTracker::current_goal()` | Direct field |
| Plan | `GoalTracker::sub_goals()` | Direct field |
| Mode | `ReActLoop::plan_mode` | Direct field |
| Health | `CircuitBreaker::tripped()`, `StormBreaker::status()` | Direct |
| Uptime | `DebugHandler::started_at.elapsed()` | Direct |
| Iteration | `ReActLoop::iteration` | Direct field |
| Recent Events | `SessionManager::messages` (last 5) | Mutex lock |
| Resource Usage | `PerfCounter`, `read_rss_kb()` | Atomic/Direct |
| Tokens | `PerfCounter::tokens_in/out` | Atomic |
| Tool Budget | `ToolBudget` | Direct field |
| Open Errors | `StormBreaker::failure_counts` | Direct field |
| Config | `RuntimeConfig` + `DaemonConfig` | Immutable copies |

---

## 7. Data Flow: Claude Attach

### 7.1 Protocol

Claude Code interacts with Aletheon purely through the Unix socket. No MCP, no file polling.

```
Claude Code
    │
    │  Bash tool: echo '{"method":"session.snapshot",...}' | nc -U /run/aletheond/aletheond.sock
    │
    ▼
Unix Socket (JSON-RPC 2.0, line-delimited)
    │
    ▼
SessionGateway::handle_method(method, params)
    │
    ├── "session.snapshot" → SnapshotBuilder::build()
    ├── "session.memory"   → SubsystemRegistry::query("memory.*")
    ├── "session.self"     → SubsystemRegistry::query("self")
    ├── "session.dasein"   → SubsystemRegistry::query("dasein")
    ├── "session.state"    → ReActLoop::export_state()
    ├── "session.param.*"  → ParamRegistry::get/list()
    ├── "session.ask"      → Injects query into session, returns response
    ├── "session.journal"  → EventJournal::query(from, limit, filter)
    ├── "session.watch"    → DebugHandler::subscribe()
    ├── "session.topic.*"  → DebugHandler::topics/subscribe()
    ├── "session.bag.*"    → DebugHandler::bag_start/stop/replay()
    ├── "session.perf"     → DebugHandler::perf()
    ├── "session.log"      → DebugHandler::log_subscribe()
    └── "session.graph"    → DebugHandler::graph/topology()
```

### 7.2 Example Debug Session

```bash
# Step 1: Get overview
$ echo '{"method":"session.snapshot","params":{},"id":1}' | nc -U /run/aletheond/aletheond.sock
# → markdown: goal, plan, health, recent events, resources, errors

# Step 2: Found tool errors — check SelfField boundary
$ echo '{"method":"session.self","params":{"layer":"boundary"},"id":2}' | nc -U /run/aletheond/aletheond.sock
# → markdown: boundary rules, risk levels, recent verdicts

# Step 3: Check specific param
$ echo '{"method":"session.param.get","params":{"key":"react.consecutive_errors"},"id":3}' | nc -U /run/aletheond/aletheond.sock
# → {"result": {"key": "react.consecutive_errors", "value": 3}}

# Step 4: Ask the Agent a question
$ echo '{"method":"session.ask","params":{"message":"Why did you call tool X 3 times?"},"id":4}' | nc -U /run/aletheond/aletheond.sock
# → markdown: Agent's reasoning

# Step 5: Check memory contents
$ echo '{"method":"session.memory","params":{"type":"core"},"id":5}' | nc -U /run/aletheond/aletheond.sock
# → markdown: all core memory blocks

# Step 6: Stream real-time events (if needed)
$ echo '{"method":"session.watch","params":{"filter":{"modules":["react_loop","tool"]}},"id":6}' | nc -U /run/aletheond/aletheond.sock
# → streaming ndjson debug events
```

---

## 8. SessionGateway Struct

### 8.1 Design

```rust
/// Session Gateway — unified facade for external debug access.
///
/// Provides both Query (structured state) and Stream (real-time events)
/// methods under the `session.*` JSON-RPC namespace.
pub struct SessionGateway {
    /// Dynamic parameter registry (new).
    param_registry: Arc<ParamRegistry>,

    /// Subsystem query registry (new).
    subsystem_registry: Arc<SubsystemRegistry>,

    /// Snapshot builder (new).
    snapshot_builder: SnapshotBuilder,

    /// Existing debug handler (reused for stream methods).
    debug_handler: Arc<DebugHandler>,

    /// Handler state for ReActLoop access.
    state: Arc<Mutex<SessionState>>,

    /// Session manager for journal + messages.
    session_manager: Arc<Mutex<SessionManager>>,

    /// Memory subsystems for queries.
    core_memory: Arc<Mutex<CoreMemory>>,
    recall_memory: Arc<Mutex<RecallMemory>>,
    fact_store: Arc<Mutex<FactStore>>,

    /// SelfField for self/dasein queries.
    self_field: Arc<Mutex<SelfField>>,
}

impl SessionGateway {
    pub fn new(
        param_registry: Arc<ParamRegistry>,
        subsystem_registry: Arc<SubsystemRegistry>,
        debug_handler: Arc<DebugHandler>,
        state: Arc<Mutex<SessionState>>,
        session_manager: Arc<Mutex<SessionManager>>,
        core_memory: Arc<Mutex<CoreMemory>>,
        recall_memory: Arc<Mutex<RecallMemory>>,
        fact_store: Arc<Mutex<FactStore>>,
        self_field: Arc<Mutex<SelfField>>,
    ) -> Self;

    /// Route a `session.*` JSON-RPC method to the appropriate handler.
    pub async fn handle_method(&self, id: &Value, method: &str, params: &Value) -> Option<Value>;
}
```

### 8.2 Method Dispatch

```rust
impl SessionGateway {
    pub async fn handle_method(&self, id: &Value, method: &str, params: &Value) -> Option<Value> {
        match method {
            // ── Query methods (new) ──
            "session.snapshot" => Some(self.handle_snapshot(id).await),
            "session.memory" => Some(self.handle_memory(id, params).await),
            "session.self" => Some(self.handle_self(id, params).await),
            "session.dasein" => Some(self.handle_dasein(id).await),
            "session.state" => Some(self.handle_state(id).await),
            "session.param.get" => Some(self.handle_param_get(id, params).await),
            "session.param.list" => Some(self.handle_param_list(id, params).await),
            "session.ask" => Some(self.handle_ask(id, params).await),
            "session.journal" => Some(self.handle_journal(id, params).await),

            // ── Stream methods (delegate to DebugHandler) ──
            "session.watch" => self.debug_handler.handle_method(id, "debug.subscribe", params).await,
            "session.topic.list" => self.debug_handler.handle_method(id, "debug.topics", params).await,
            "session.topic.echo" => self.debug_handler.handle_method(id, "debug.subscribe", params).await,
            "session.bag.record" => self.debug_handler.handle_method(id, "debug.bag_start", params).await,
            "session.bag.stop" => self.debug_handler.handle_method(id, "debug.bag_stop", params).await,
            "session.bag.play" => self.debug_handler.handle_method(id, "debug.bag_replay", params).await,
            "session.perf" => self.debug_handler.handle_method(id, "debug.perf", params).await,
            "session.log" => self.debug_handler.handle_method(id, "debug.log_subscribe", params).await,
            "session.graph" => self.debug_handler.handle_method(id, "debug.graph", params).await,

            _ => None,
        }
    }
}
```

---

## 9. Integration Points

### 9.1 RequestHandler Changes

In `handler/mod.rs`, the `Handle` implementation adds SessionGateway dispatch:

```rust
// Before (current):
if method.starts_with("debug.") {
    if let Some(response) = self.debug_handler.handle_method(&id, method, &params).await {
        return response;
    }
}

// After:
if let Some(response) = self.session_gateway.handle_method(&id, method, &params).await {
    return response;
}
// Fallback: handle_debug and handle_id remain for backward compat
if method.starts_with("debug.") {
    if let Some(response) = self.debug_handler.handle_method(&id, method, &params).await {
        return response;
    }
}
```

### 9.2 RequestHandler Construction

Add a `session_gateway: Arc<SessionGateway>` field, constructed during `RequestHandler::new()`.

### 9.3 DebugHandler Changes

- `handle_param_get` / `handle_param_list` — replaced by `ParamRegistry`, but kept for backward compatibility
- `debug_handler.rs` line 528-583: hard-coded params remain for `debug.param_*` but are NOT extended

### 9.4 Tracepoint Expansion

Add new builtin tracepoints for Session Gateway operations:

```rust
// In debug_handler.rs builtin tracepoints:
("session.query", "debug", DebugLevel::Info, "Session Gateway query"),
("session.ask", "debug", DebugLevel::Info, "External agent ask"),
```

---

## 10. Responsibility Boundary

In accordance with `debug.loop.md`:

**Session Gateway (Aletheon) MAY:**
- Expose runtime state (snapshot, memory, self, dasein, state, params)
- Expose event streams (watch, topic, log)
- Answer context-aware questions (ask)
- Export history (journal, bag, replay)

**External Agent (Claude/Codex) MAY:**
- Inspect runtime state
- Inspect memory contents
- Ask questions to the Agent
- Generate bug reports and patch proposals

**External Agent MUST NOT (without human approval):**
- Modify architecture
- Merge patches
- Disable security
- Bypass permissions
- Rewrite kernel components

**The Session Gateway does NOT provide:**
- Any write/modify methods (no `session.set_param`, no `session.modify_memory`)
- Any self-modification capabilities
- Any bypass of `SelfField.review()` or `PermissionManager`

---

## 11. Files Changed

| File | Action | Purpose |
|---|---|---|
| `crates/runtime/src/core/session_gateway.rs` | **CREATE** | `SessionGateway` struct + method dispatch |
| `crates/runtime/src/core/session_gateway/param_registry.rs` | **CREATE** | `ParamRegistry` + `ParamEntry` |
| `crates/runtime/src/core/session_gateway/snapshot.rs` | **CREATE** | `SnapshotBuilder` |
| `crates/runtime/src/core/session_gateway/subsystem_query.rs` | **CREATE** | `SubsystemQuery` trait + `SubsystemRegistry` |
| `crates/runtime/src/core/session_gateway/markdown.rs` | **CREATE** | Markdown formatting utilities |
| `crates/runtime/src/core/mod.rs` | Modify | Add `pub mod session_gateway;` |
| `crates/runtime/src/impl/daemon/handler/mod.rs` | Modify | Add `session_gateway` field, dispatch `session.*` methods, register params in `new()` |
| `crates/runtime/src/impl/daemon/debug_handler.rs` | Modify | Deprecation comment on param methods, add session tracepoints |
| `crates/runtime/src/impl/session/journal.rs` | Modify | Add `query()` method for `session.journal` |
| `crates/runtime/src/core/react_loop/mod.rs` | Modify | Add `export_state()` and param-compatible getters |
| `crates/dasein/src/core/mod.rs` | Modify | Add param registration + `impl SubsystemQuery` for SelfField |
| `crates/dasein/src/dasein/mod.rs` | Modify | Add param registration + `impl SubsystemQuery` for DaseinModule |
| `crates/runtime/src/impl/memory/core_memory.rs` | Modify | Add `impl SubsystemQuery` |
| `crates/runtime/src/impl/memory/recall_memory.rs` | Modify | Add `impl SubsystemQuery` |
| `crates/runtime/src/impl/memory/fact_store/mod.rs` | Modify | Add `impl SubsystemQuery` |
| `crates/interact/src/tui/debug.rs` | Modify | Add `session snapshot/memory/self/dasein/state/ask/journal` subcommands |
| `crates/runtime/src/core/react_loop/goal_tracker.rs` | Modify | Add public getters |
| `crates/runtime/src/core/react_loop/circuit_breaker.rs` | Modify | Add `status()` method |
| `crates/runtime/src/core/react_loop/tool_budget.rs` | Modify | Add `remaining()` getter |
| `crates/runtime/src/core/storm_breaker.rs` | Modify | Add `status()` method |
| `docs/plans/2026-07-03-session-gateway-design.md` | **CREATE** | This design document |

---

## 12. Implementation Phases

### Phase A: ParamRegistry + Infrastructure (2-3 files)

1. Create `ParamRegistry` with `declare/get/list/dump`
2. Register initial params in `RequestHandler::new()`
3. Wire `session.param.get/list` in `SessionGateway::handle_method()`
4. Keep `debug.param_*` for backward compat

### Phase B: SnapshotBuilder (2-3 files)

1. Create `SnapshotBuilder` aggregating from GoalTracker, PerfCounter, CircuitBreaker, etc.
2. Add public getters to GoalTracker, CircuitBreaker, ToolBudget, StormBreaker
3. Wire `session.snapshot` in SessionGateway
4. Add `aletheon debug session snapshot` CLI subcommand

### Phase C: SubsystemQuery (4-6 files)

1. Define `SubsystemQuery` trait + `SubsystemRegistry`
2. Implement for CoreMemory, RecallMemory, FactStore, SelfField, DaseinModule
3. Wire `session.memory`, `session.self`, `session.dasein`, `session.state`
4. Add `aletheon debug session memory/self/dasein/state` CLI subcommands

### Phase D: Stream Unification (1-2 files)

1. Route `session.watch/topic/bag/perf/log/graph` to DebugHandler
2. Add session tracepoints
3. Add `aletheon debug session watch` CLI subcommand

### Phase E: Session Ask + Journal (2-3 files)

1. Implement `session.ask` — inject query into session as system message
2. Add `EventJournal::query()` for `session.journal`
3. Add `aletheon debug session ask/journal` CLI subcommands

---

## 13. Verification

```bash
# Build check
cargo build -p runtime

# Existing tests must still pass
cargo test -p runtime --lib  # currently 554 pass

# New tests
cargo test -p runtime --lib -- 'param_registry'
cargo test -p runtime --lib -- 'session_gateway'
cargo test -p runtime --lib -- 'snapshot_builder'
cargo test -p runtime --lib -- 'subsystem_query'

# Full workspace build
cargo build --workspace
```

---

## 14. Risks

| Risk | Mitigation |
|---|---|
| SessionGateway holds many Arc references, construction is large | Same pattern as existing RequestHandler (39 fields). Acceptable. |
| ParamRegistry getters are called without locking strategy | Each getter handles its own locking. Getters are called synchronously in the JSON-RPC handler. |
| Markdown format is not machine-parseable | Query responses are markdown for human/Claude reading. If machine parsing is needed later, add `?format=json` parameter. |
| `session.ask` could be abused to modify agent behavior | Responses are read-only. The injected message is system-level (not user), and does not trigger tool execution. |
| Debug system already has 2 namespaces (debug.* + session.*) | debug.* is frozen (no new methods). session.* is the canonical namespace going forward. Migration path is clear. |
