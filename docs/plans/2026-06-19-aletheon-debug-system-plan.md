# Aletheon Debug System — Implementation Plan

**Design doc**: `docs/plans/2026-06-19-aletheon-debug-system-design.md`
**Date**: 2026-06-19

## Phase 1: ABI Tracepoint Interface

### Task 1.1: Create debug types
- File: `aletheon-abi/src/debug.rs` (NEW)
- Types: DebugLevel, Tracepoint, DebugEvent, DebugSink trait
- Macros: tracepoint!(), trace!()
- Tests: Unit tests for serialization, level ordering

### Task 1.2: Register in ABI lib
- File: `aletheon-abi/src/lib.rs`
- Add: `pub mod debug;`
- Tests: `cargo test -p aletheon-abi`

## Phase 2: Bus Debug Hook

### Task 2.1: Create DebugBusHook
- File: `aletheon-comm/src/impl/debug_bus.rs` (NEW)
- Types: DebugBusHook, EventFilter, EventRecorder, PerfCounter
- Tests: Unit tests for filter, recorder, perf counter

### Task 2.2: Wire into CommunicationBus
- File: `aletheon-comm/src/impl/communication_bus.rs`
- Add: `with_debug_hook()` method
- Integration: Call `hook.on_event()` in publish path
- Tests: Integration test with EventBus

### Task 2.3: Register in comm lib
- File: `aletheon-comm/src/lib.rs`
- Add: `pub mod debug_bus;` and re-exports
- Tests: `cargo test -p aletheon-comm`

## Phase 3: Daemon Debug API

### Task 3.1: Create debug handler
- File: `aletheon-runtime/src/impl/daemon/debug_handler.rs` (NEW)
- Methods: debug.subscribe, debug.topics, debug.node_info, debug.bag_*, debug.perf, debug.trace_*
- Tests: Unit tests for each method

### Task 3.2: Register in daemon handler
- File: `aletheon-runtime/src/impl/daemon/handler.rs`
- Add: Debug handler instance
- Integration: Route debug.* methods to debug handler
- Tests: JSON-RPC integration tests

### Task 3.3: Wire debug hook to CommunicationBus
- File: `aletheon-runtime/src/impl/daemon/handler.rs`
- Add: Create DebugBusHook and attach to CommunicationBus
- Tests: End-to-end event flow test

## Phase 4: CLI Tools

### Task 4.1: Create debug subcommand
- File: `crates/binaries/aletheon-cli/src/debug.rs` (NEW)
- Commands: topic, node, bag, perf, trace
- Tests: Unit tests for argument parsing

### Task 4.2: Implement topic commands
- `aletheon debug topic list` — call debug.topics
- `aletheon debug topic echo` — subscribe and print
- Tests: Integration test with daemon

### Task 4.3: Implement node command
- `aletheon debug node info` — call debug.node_info
- Tests: Integration test

### Task 4.4: Implement bag commands
- `aletheon debug bag record` — call debug.bag_start, stream events
- `aletheon debug bag play` — call debug.bag_replay
- `aletheon debug bag info` — read bag metadata
- Tests: Record/play round-trip test

### Task 4.5: Implement perf command
- `aletheon debug perf` — call debug.perf
- Tests: Integration test

### Task 4.6: Implement trace commands
- `aletheon debug trace start/stop/status`
- Tests: Integration test

## Phase 5: Integration + Docs

### Task 5.1: End-to-end tests
- File: `tests/debug_e2e.rs`
- Test: Record session → replay → verify events
- Test: CLI topic echo → verify output
- Test: Perf stats → verify numbers

### Task 5.2: Documentation
- File: `docs/debug.md`
- Content: Usage guide, examples, troubleshooting

### Task 5.3: Performance validation
- Verify: Debug overhead < 1% when tracing is off
- Benchmark: Event recording throughput

## Dependencies

```
Phase 1 (ABI) → Phase 2 (Bus) → Phase 3 (Daemon API) → Phase 4 (CLI) → Phase 5 (Integration)
```

Each phase depends on the previous one. Within each phase, tasks can be parallelized.

## Validation

After all phases:
1. `cargo test` — all tests pass
2. `aletheon debug topic list` — shows tracepoints
3. `aletheon debug topic echo` — shows real-time events
4. `aletheon debug bag record/play` — round-trip works
5. `aletheon debug node info` — shows daemon status
6. `aletheon debug perf` — shows stats
7. Debug overhead < 1% when off
