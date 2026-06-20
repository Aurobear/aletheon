# Aletheon Debug System — Design Document

**Date**: 2026-06-19
**Status**: Draft
**Inspiration**: Linux kernel (tracepoints/ftrace/debugfs), ROS (rostopic/rosnode/rosbag)

## 1. Problem

Aletheon lacks debugging infrastructure. There's no way to:
- Inspect event flow between subsystems in real time
- Record and replay sessions for debugging
- Check daemon/session/agent status programmatically
- Profile token usage, latency, and throughput

The existing `aletheon-comm` EventBus and `aletheon-abi` EventType types are rich but not exposed to any debugging tooling.

## 2. Architecture

Three-layer design inspired by Linux kernel's tracepoint → ftrace → debugfs architecture:

```
┌─────────────────────────────────────────────────────────┐
│  Layer 3: CLI Tools                                      │
│  aletheon debug topic echo/list                          │
│  aletheon debug node info                                │
│  aletheon debug bag record/play                          │
│  aletheon debug perf/trace                               │
└──────────────────────┬──────────────────────────────────┘
                       │ JSON-RPC (debug.*)
┌──────────────────────┴──────────────────────────────────┐
│  Layer 2: Bus Debug Hook                                 │
│  DebugBusHook — observes EventBus.publish()              │
│  EventRecorder — rosbag-style recording                  │
│  EventFilter — level/module/type filtering               │
│  PerfCounter — token/latency/throughput stats            │
└──────────────────────┬──────────────────────────────────┘
                       │ DebugSink trait
┌──────────────────────┴──────────────────────────────────┐
│  Layer 1: ABI Tracepoint Interface                       │
│  Tracepoint — static probe definition                    │
│  DebugEvent — unified event format                       │
│  DebugSink — event receiver trait                        │
│  DebugLevel — Off/Error/Warn/Info/Debug/Trace            │
└─────────────────────────────────────────────────────────┘
```

## 3. Layer 1: ABI Tracepoint Interface

### 3.1 Debug Level

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DebugLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}
```

### 3.2 Tracepoint Definition

```rust
/// Static probe — registered at compile time.
#[derive(Debug, Clone)]
pub struct Tracepoint {
    pub name: &'static str,        // "react_loop.iteration"
    pub module: &'static str,      // "runtime"
    pub level: DebugLevel,
    pub description: &'static str,
}

/// Macro for declaring tracepoints.
/// tracepoint!(runtime, Debug, "react_loop.iteration", "ReAct loop iteration started");
#[macro_export]
macro_rules! tracepoint {
    ($module:ident, $level:ident, $name:expr, $desc:expr) => {
        static TP: $crate::debug::Tracepoint = $crate::debug::Tracepoint {
            name: $name,
            module: stringify!($module),
            level: $crate::debug::DebugLevel::$level,
            description: $desc,
        };
    };
}
```

### 3.3 Debug Event

```rust
/// Unified debug event format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugEvent {
    pub ts: u64,                    // Unix millis
    pub tracepoint: String,         // "react_loop.iteration"
    pub module: String,             // "runtime"
    pub level: DebugLevel,
    pub data: serde_json::Value,    // Arbitrary payload
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub thread_id: Option<u64>,
}
```

### 3.4 Debug Sink Trait

```rust
/// Receives debug events — implemented by bus hook, CLI subscriber, recorder.
#[async_trait]
pub trait DebugSink: Send + Sync {
    async fn emit(&self, event: DebugEvent);
    fn should_trace(&self, tp: &Tracepoint) -> bool;
}
```

## 4. Layer 2: Bus Debug Hook

### 4.1 DebugBusHook

```rust
/// Observer attached to CommunicationBus.
/// Called on every EventBus.publish().
pub struct DebugBusHook {
    sinks: Vec<Box<dyn DebugSink>>,
    filter: EventFilter,
    recorder: Option<EventRecorder>,
    perf: PerfCounter,
}
```

### 4.2 Event Filter

```rust
pub struct EventFilter {
    pub min_level: DebugLevel,
    pub modules: Option<HashSet<String>>,     // None = all
    pub tracepoints: Option<HashSet<String>>, // None = all
}
```

### 4.3 Event Recorder (rosbag equivalent)

```rust
/// Records events to a file for later replay.
pub struct EventRecorder {
    path: PathBuf,
    file: File,
    buffer: VecDeque<DebugEvent>,
    max_buffer: usize,          // Flush threshold
    started_at: Instant,
    event_count: u64,
}

impl EventRecorder {
    pub async fn start(path: &Path) -> anyhow::Result<Self>;
    pub async fn record(&mut self, event: DebugEvent);
    pub async fn stop(self) -> anyhow::Result<RecordingMeta>;
    pub async fn replay(path: &Path, sink: &dyn DebugSink, speed: f64) -> anyhow::Result<()>;
}
```

### 4.4 Performance Counter

```rust
/// Tracks token usage, latency, throughput.
pub struct PerfCounter {
    tokens_in: AtomicU64,
    tokens_out: AtomicU64,
    turn_count: AtomicU64,
    turn_latencies: VecDeque<Duration>,
    tool_calls: HashMap<String, u64>,
    errors: HashMap<String, u64>,
}
```

## 5. Layer 3: Daemon Debug API

### 5.1 JSON-RPC Methods

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `debug.subscribe` | `{filter?}` | SSE stream | Subscribe to debug events |
| `debug.unsubscribe` | `{id}` | `{ok}` | Unsubscribe |
| `debug.topics` | `{}` | `[{name, module, level, desc}]` | List tracepoints |
| `debug.node_info` | `{session_id?}` | `{...}` | Daemon/session/agent status |
| `debug.bag_start` | `{path?, filter?}` | `{id}` | Start recording |
| `debug.bag_stop` | `{id}` | `{path, events, duration}` | Stop recording |
| `debug.bag_replay` | `{path, speed?}` | `{events, duration}` | Replay recording |
| `debug.perf` | `{interval?}` | `{tokens, latency, ...}` | Performance stats |
| `debug.trace_start` | `{module?, level?}` | `{ok}` | Enable tracing |
| `debug.trace_stop` | `{}` | `{ok}` | Disable tracing |

### 5.2 SSE Stream Format

```
event: debug_event
data: {"ts":1234567890,"tracepoint":"react_loop.iteration","module":"runtime","level":"debug","data":{...}}

event: perf_update
data: {"tokens_in":1500,"tokens_out":300,"latency_ms":2100}
```

## 6. CLI Tools

### 6.1 aletheon debug topic

```bash
# List all registered tracepoints
aletheon debug topic list
# OUTPUT:
#   react_loop.iteration    [runtime]  debug  ReAct loop iteration started
#   tool.dispatch           [body]     info   Tool call dispatched
#   memory.stored           [memory]   info   Fact stored to memory

# Echo events in real time
aletheon debug topic echo
# OUTPUT:
#   [12:34:56.789] DEBUG runtime.react_loop.iteration turn=5
#   [12:34:57.123] INFO  body.tool.dispatch tool=read_file args={...}
#   [12:34:57.456] INFO  body.tool.result tool=read_file ok (230ms)

# With filters
aletheon debug topic echo --filter module=runtime --level debug
aletheon debug topic echo --filter tracepoint=tool.*
```

### 6.2 aletheon debug node

```bash
# Daemon info
aletheon debug node info
# OUTPUT:
#   Daemon:    PID 12345, uptime 2h30m
#   Provider:  deepseek (deepseek-v4-flash)
#   Sessions:  1 active, 5 total
#   Memory:    1.2GB RSS, 45MB event log
#   Tokens:    150k in, 30k out (last hour)

# Session info
aletheon debug node info --session <id>
```

### 6.3 aletheon debug bag

```bash
# Record events
aletheon debug bag record -o session.bag --filter module=runtime
# ^C to stop
# OUTPUT: Recorded 1234 events in 45.2s → session.bag (2.3MB)

# Replay events
aletheon debug bag play session.bag
aletheon debug bag play session.bag --speed 2.0

# Inspect bag
aletheon debug bag info session.bag
# OUTPUT:
#   Events:   1234
#   Duration: 45.2s
#   Modules:  runtime(800), body(300), memory(134)
#   Size:     2.3MB
```

### 6.4 aletheon debug perf

```bash
# Real-time performance
aletheon debug perf
# OUTPUT:
#   Tokens:    150k in / 30k out (180k total)
#   Latency:   avg 2.1s, p50 1.8s, p99 5.2s
#   Turns:     45 (2.3/min)
#   Tools:     read_file(12), bash(8), write_file(5)
#   Errors:    2 (tool timeout)

# Continuous monitoring
aletheon debug perf --interval 5
```

## 7. Phased Implementation

### Phase 1: ABI Tracepoint Interface
- File: `aletheon-abi/src/debug.rs`
- Types: DebugLevel, Tracepoint, DebugEvent, DebugSink
- Macros: tracepoint!(), trace!()
- Tests: Unit tests for types and macros

### Phase 2: Bus Debug Hook
- File: `aletheon-comm/src/impl/debug_bus.rs`
- Types: DebugBusHook, EventFilter, EventRecorder, PerfCounter
- Integration: Wire into CommunicationBus
- Tests: Unit tests + integration with EventBus

### Phase 3: Daemon Debug API
- File: `aletheon-runtime/src/impl/daemon/debug_handler.rs`
- Methods: debug.subscribe, debug.topics, debug.node_info, debug.bag_*, debug.perf, debug.trace_*
- Integration: Register in daemon handler
- Tests: JSON-RPC integration tests

### Phase 4: CLI Tools
- Files: `aletheon-debug/` crate or `aletheon debug` subcommand
- Commands: topic, node, bag, perf, trace
- Integration: Connect to daemon via socket
- Tests: CLI integration tests

### Phase 5: Integration + Docs
- End-to-end tests
- Documentation
- Migration guide

## 8. File Changes

| Phase | Files | Description |
|-------|-------|-------------|
| 1 | `aletheon-abi/src/debug.rs`, `aletheon-abi/src/lib.rs` | ABI types |
| 2 | `aletheon-comm/src/impl/debug_bus.rs`, `aletheon-comm/src/lib.rs` | Bus hook |
| 3 | `aletheon-runtime/src/impl/daemon/debug_handler.rs` | Daemon API |
| 4 | `crates/binaries/aletheon-cli/src/debug.rs` or `aletheon-debug/` | CLI tools |
| 5 | `tests/debug_*.rs`, `docs/debug.md` | Tests + docs |

## 9. Success Criteria

1. `aletheon debug topic list` shows all registered tracepoints
2. `aletheon debug topic echo` shows real-time event stream
3. `aletheon debug bag record/play` records and replays sessions
4. `aletheon debug node info` shows daemon/session/agent status
5. `aletheon debug perf` shows token/latency/throughput stats
6. All existing tests pass (no regression)
7. Debug overhead < 1% when tracing is off
