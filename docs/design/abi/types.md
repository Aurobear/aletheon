# ABI: Shared Types, Traits, and Interfaces

> Migrated from `docs/design/shared/types.md`, `docs/design/shared/traits.md`, `docs/design/shared/interfaces.md` — code paths updated to aletheon-* crate structure

**Module:** aletheon-abi
**Last Updated:** 2026-06-14

---

## 1. Shared Types

### Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| ContentBlock | Implemented | `aletheon-abi/src/message.rs` | Text, ToolUse, ToolResult, Image |
| Message | Implemented | `aletheon-abi/src/message.rs` | Conversation message wrapper |
| ToolCall | Implemented | `aletheon-abi/src/message.rs` | Tool invocation request |
| ToolResult | Implemented | `aletheon-abi/src/tool.rs` | Tool execution result |
| PerceptionEvent | Implemented | `aletheon-self/src/impl/perception/event.rs` | System event type |
| AgentError | Implemented | `aletheon-abi/src/error.rs` | Typed error with severity/category |

### 1.1 Message Types

- **ContentBlock** — Content-block message protocol (Anthropic SDK compatible), with Text, ToolUse, ToolResult, Image variants
- **Message** — Conversation message wrapper (role: System/User/Assistant + content: Vec<ContentBlock>)
- Code location: `aletheon-abi/src/message.rs`

### 1.2 Tool Calls

> **Canonical ToolResult definition** — other docs MUST reference this file, not redefine these types.

- **ToolCall** — Tool invocation request (id, name, input)
- **ToolResult** — Tool execution result (tool_call_id, content: Vec<ToolContent>, is_error, exit_code, metadata)
- **ToolContent** — Output content variant: Text / Image / Binary
- **ToolResultMeta** — Metadata (execution_time_ms, tokens_used, truncated)
- Code location: `aletheon-abi/src/message.rs`, `aletheon-abi/src/tool.rs`

### 1.3 Perception Events

- **PerceptionEvent** — System event (id, source, kind, payload, priority, timestamp)
- **EventSource** — Ebpf / Proc / Sys / Journald / Inotify / Udev / DBus
- **EventKind** — FileCreated/Modified/Deleted, ProcessStarted/Exited, NetworkConnect/Disconnect, ServiceStarted/Failed, DeviceAdded/Removed, CpuPressure/MemoryPressure/DiskPressure
- Code location: `aletheon-self/src/impl/perception/event.rs`

### 1.4 Error Types

- **AgentError** — Typed error (category, severity, message, source, context)
- **ErrorSeverity** — Warning / Error / Critical / Fatal
- **ErrorCategory** — Tool / Llm / Session / Memory / Permission / System
- Code location: `aletheon-abi/src/error.rs`

### Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| ContentBlock / Message / ToolCall | `aletheon-abi/src/message.rs` | `ContentBlock`, `Message`, `ToolCall` |
| ToolResult / ToolContent | `aletheon-abi/src/tool.rs` | `ToolResult`, `ToolContent`, `ToolResultMeta` |
| PerceptionEvent | `aletheon-self/src/impl/perception/event.rs` | `PerceptionEvent`, `EventSource`, `EventKind` |

---

## 2. Shared Traits

> **This file is the CANONICAL definition. Other docs MUST reference this file, not redefine these traits.**

### Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| LlmProvider | Implemented | `aletheon-brain/src/impl/inference/provider.rs` | Provider trait with complete/complete_stream |
| Tool | Implemented | `aletheon-abi/src/tool.rs` | Includes permission_level(), exposure(), concurrency_class() |
| PlatformAdapter | Implemented | `aletheon-body/src/impl/platform/adapter.rs`, `aletheon-body/src/impl/platform/linux.rs`, `aletheon-body/src/impl/platform/android.rs` | Linux (systemd/D-Bus) + Android (getprop/dumpsys) |
| MemoryStore | Planned | — | Memory uses different API than this trait |

### 2.1 LLM Provider

**LlmProvider** — LLM provider interface, supporting complete (sync) and complete_stream (streaming) inference modes.
- Code location: `aletheon-brain/src/impl/inference/provider.rs`
- Methods: complete, complete_stream, name, max_context_length

### 2.2 Tool

> **Canonical definition** — superset of all fields from tool-system, platform-adapter, and loop-detector docs.

**Tool** — Unified tool interface, including name, description, input_schema, permission_level (L0-L3), needs_sandbox, exposure (ToolExposure), concurrency_class (ConcurrencyClass), execute.
- Code location: `aletheon-abi/src/tool.rs`
- `ToolExposure` and `ConcurrencyClass` enums defined in `aletheon-body/src/impl/tools/`

### 2.3 PlatformAdapter

> Implemented — Linux and Android adapters complete.

**PlatformAdapter** — Platform adapter interface, covering IPC (send/recv), process lifecycle (spawn/kill), filesystem (read/write/watch), permissions (check/elevate).
- Code location: `aletheon-body/src/impl/platform/adapter.rs` (trait), `aletheon-body/src/impl/platform/linux.rs` (Linux/D-Bus), `aletheon-body/src/impl/platform/android.rs` (Android stub)

### 2.4 MemoryStore

> Planned — complete design preserved.

**MemoryStore** — Memory storage interface, including read_core, write_core, search_recall, search_archival, record_outcome.
- Code location: No implementation yet (Memory uses different API)

### Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| LlmProvider trait | `aletheon-brain/src/impl/inference/provider.rs` | `LlmProvider`, `LlmRequest`, `LlmResponse` |
| Tool trait | `aletheon-abi/src/tool.rs` | `Tool`, `ToolExposure`, `ConcurrencyClass` |
| PlatformAdapter trait | `aletheon-body/src/impl/platform/adapter.rs` | `PlatformAdapter` |
| Linux adapter | `aletheon-body/src/impl/platform/linux.rs` | `LinuxPlatformAdapter` (systemd/D-Bus) |
| Android adapter | `aletheon-body/src/impl/platform/android.rs` | `AndroidPlatformAdapter` (getprop/dumpsys) |

---

## 3. Inter-Module Interfaces

> Module boundary communication contracts.

### Implementation Status

> These are interface contracts defining module boundaries. Not all are fully implemented.
> Status reflects whether the interface is exercised in practice.

| Interface | Status | Notes |
|-----------|--------|-------|
| CognitiveEngine <-> ToolSystem | Implemented | Engine calls tools via ToolRegistry |
| PerceptionEngine -> CognitiveEngine | Implemented | PerceptionBridge -> injection_tx -> engine.drain_perceptions() wired before each turn |
| CognitiveEngine <-> MemorySystem | Implemented | Core memory reads/writes during loop |
| Security -> ToolSystem | Implemented | Policy checks before tool execution |
| Orchestration -> ToolSystem | Implemented | DelegateTool as tool call |

### 3.1 Cognitive Engine <-> Tool System

```
Cognitive Engine calls tools:
  LlmResponse.tool_calls -> ToolRegistry.execute() -> ToolResult -> messages.push()

Tool result feedback:
  ToolResult -> check is_error -> decide retry/skip/terminate
```

### 3.2 Perception Engine -> Cognitive Engine

```
PerceptionEvent -> EventAggregator -> filter/dedup/aggregate
  -> high priority: inject directly into cognitive engine message queue
  -> low priority: write to Core Memory system_state block
  -> event stats: update observability metrics
```

### 3.3 Security Engine -> Tool System

```
Before Tool.execute():
  -> SecurityEngine.check_permission(tool, input) -> Allow/Deny/Confirm
  -> LoopDetector.record_call(tool) -> whether loop detection triggers
  -> WritableRoot.check_path(input) -> path allowed?

After Tool.execute():
  -> AuditLog.record(tool, input, result)
```

### 3.4 Orchestration Engine -> Sub-Agent

```
Orchestrator.create_sub_agent(config)
  -> AgentRegistry.register(agent_info)
  -> create independent Channel for sub-agent
  -> sub-agent runs ReAct loop
  -> result returns to parent agent via Channel
```

### 3.5 Proactive Behavior Engine -> Orchestration Engine

> Design aspiration only — ProactiveGoal, GoalQueue, IdleScheduler have NO code.

```
ProactiveGoal -> GoalQueue.push(goal)
  -> IdleScheduler decides when to execute
  -> Orchestrator.execute(goal) -> uses SingleAgent strategy
```

### 3.6 Self-Learning Loop -> Memory System

> Code exists but not wired — `learning/` module (outcome, pattern, rule) is standalone;
> not integrated into engine or handler.

```
ToolResult + UserFeedback -> OutcomeRecorder.record()
  -> store in Recall Memory (SQLite)
  -> PatternExtractor periodically analyzes
  -> LearnRule -> write to Core Memory (learned_rules block)
```

### Implementation Summary

| Interface | Code Location | Notes |
|-----------|---------------|-------|
| Engine -> ToolRegistry | `aletheon-runtime/src/impl/engine/cognitive_loop.rs`, `aletheon-body/src/impl/tools/` | Engine calls tools via `ToolRegistry::execute()` |
| Security -> ToolRunner | `aletheon-body/src/impl/security/policy.rs`, `aletheon-body/src/impl/security/runner.rs` | Policy + LoopDetector checks before execution |
| DelegateTool | `aletheon-runtime/src/impl/orchestration/` | Delegation as tool call |
| Perception -> Engine | `aletheon-self/src/impl/perception/bridge.rs`, `aletheon-runtime/src/impl/engine/cognitive_loop.rs` | PerceptionBridge wired via injection_tx |
