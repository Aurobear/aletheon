# Fabric Crate — Shared Contracts and Communication

> Code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `fabric`
**Purpose:** Core trait definitions and shared types for the Aletheon persistent self-evolving runtime. Contains **zero implementations** — only interfaces, like Linux kernel header files defining the contract between subsystems.

---

## Module Index

### include/ — Subsystem trait contracts (zero implementations)

| Module | File | Purpose |
|--------|------|---------|
| `subsystem` | `fabric/src/include/subsystem.rs` | `Subsystem` trait, `SubsystemHealth`, `SubsystemContext`, `Version` |
| `cognit` | `fabric/src/include/cognit.rs` | `CognitOps` trait, `Plan`, `PlanStep`, `CostEstimate`, `ExecutionResult`, `Reflection`, `Critique`, `LearnedRule`, `Experience`, `Observation` |
| `agora` | `fabric/src/include/agora.rs` | `AgoraOps` trait, shared cognitive workspace interface |
| `body` | `fabric/src/include/body.rs` | `BodyRuntime` trait, `Action`, `ActionResult` |
| `memory` | `fabric/src/include/memory.rs` | `MemoryBackend`, `MemoryEntry`, `MemoryHandle`, `MemoryQuery`, `MemoryType`, `MemoryFilter`, `CompactStrategy`, `CompactResult`, `MemoryStats` |
| `self_field` | `fabric/src/include/self_field.rs` | `SelfFieldOps` trait, `Verdict`, `Intent`, `Identity`, `Care`, `Conflict`, `Resolution`, `MutationIntent` |
| `meta` | `fabric/src/include/meta.rs` | `MetaRuntimeOps` trait, `RuntimeCandidate`, `TestResult`, `Evaluation`, `MigrationResult` |
| `runtime` | `fabric/src/include/runtime.rs` | `RuntimeOps` trait, `AgentInfo`, `AgentStatus`, `ScheduledTask`, `ScheduleKind`, `StepResult` |
| `plugin` | `fabric/src/include/plugin.rs` | `PluginOps` trait, plugin lifecycle interface |
| `process` | `fabric/src/include/process.rs` | `ProcessOps` trait, process management interface |
| `compaction` | `fabric/src/include/compaction.rs` | `CompactorTrait`, tool-output pruner |
| `turn` | `fabric/src/include/turn.rs` | `TurnOps` trait, turn lifecycle interface |
| `space` | `fabric/src/include/space.rs` | `SpaceOps` trait, workspace management |

### types/ — Shared data models

| Module | File | Purpose |
|--------|------|---------|
| `capability` | `fabric/src/types/capability.rs` | `Capability`, `CapabilitySet`, `PermissionLevel` |
| `message` | `fabric/src/types/message.rs` | `Message`, `ContentBlock`, `Role`, `ImageSource`, `ToolCall` |
| `tool` | `fabric/src/types/tool.rs` | `Tool` trait, `ToolResult`, `ToolResultMeta`, `ToolContext`, `PermissionLevel` (L0-L3) |
| `sandbox` | `fabric/src/types/sandbox.rs` | `SandboxBackend`, `SandboxConfig`, `SandboxResult`, `SandboxCapabilities`, `IsolationLevel` |
| `genome` | `fabric/src/types/genome.rs` | `Genome` model (topology, identity, boundary, care, memory, mutation, lifecycle) |
| `paths` | `fabric/src/types/paths.rs` | Standard filesystem paths |
| `llm_types` | `fabric/src/types/llm_types.rs` | `ToolDefinition`, `LlmProvider` trait |
| `context` | `fabric/src/types/context.rs` | `Context`, `TraceState` |
| `agent` | `fabric/src/types/agent.rs` | `AgentId`, agent identity types |
| `permission` | `fabric/src/types/permission.rs` | Permission model and authority types |
| `hook` | `fabric/src/types/hook.rs` | `HookEvent`, hook lifecycle types |
| `turn` | `fabric/src/types/turn.rs` | Turn metadata and state types |
| `operation` | `fabric/src/types/operation.rs` | Operation abstractions for tool calls |
| `resource` | `fabric/src/types/resource.rs` | Resource allocation and tracking |
| `objective` | `fabric/src/types/objective.rs` | Goal/objective definitions |
| `evidence` | `fabric/src/types/evidence.rs` | Evidence and reasoning artifacts |
| `grounding` | `fabric/src/types/grounding.rs` | Grounding context for tool execution |
| `process` | `fabric/src/types/process.rs` | Process lifecycle types |
| `space` | `fabric/src/types/space.rs` | Workspace space types |
| `time` | `fabric/src/types/time.rs` | Time-related types |
| `vision` | `fabric/src/types/vision.rs` | Vision capability types |
| `admission` | `fabric/src/types/admission.rs` | Admission control types |

### ipc/ — Inter-process communication layer

| Module | File | Purpose |
|--------|------|---------|
| `communication_bus` | `fabric/src/ipc/bus/` | CommunicationBus (replaces EventBus) — single event bus system |
| `bus_handle` | `fabric/src/ipc/bus_handle.rs` | Bus handle for clients |
| `envelope` | `fabric/src/ipc/envelope.rs` | Envelope (original message wrapper) |
| `envelope_v2` | `fabric/src/ipc/envelope_v2.rs` | EnvelopeV2 with schema enforcement |
| `ipc_msg` | `fabric/src/ipc/ipc_msg.rs` | IpcMessage types |
| `ipc_types` | `fabric/src/ipc/ipc_types.rs` | `IpcBackend`, `IpcPreference`, `IpcProbeError`, `AgentMessage`, `AgentId`, `MessageType`, `IpcPriority` |
| `mailbox` | `fabric/src/ipc/mailbox.rs` | Mailbox for agent message delivery |
| `protocol` | `fabric/src/ipc/protocol.rs` | Wire protocol definitions |
| `stream` | `fabric/src/ipc/stream.rs` | Stream abstraction |
| `backends` | `fabric/src/ipc/backends/` | IPC backend implementations |
| `transport` | `fabric/src/ipc/transport/` | Transport layer abstractions |

### events/ — Event system

| Module | File | Purpose |
|--------|------|---------|
| `event_log` | `fabric/src/events/event_log.rs` | Event log storage and query |
| `evolution` | `fabric/src/events/evolution.rs` | Event schema evolution |
| `routing_policy` | `fabric/src/events/routing_policy.rs` | Event routing rules |
| `subscription` | `fabric/src/events/subscription.rs` | Event subscription management |
| `ui_event` | `fabric/src/events/ui_event.rs` | UI event types |

### dasein/ — Self-field types

| Module | File | Purpose |
|--------|------|---------|
| `context` | `fabric/src/dasein/context.rs` | DaseinContext for self-field state |
| `event` | `fabric/src/dasein/event.rs` | Dasein events |
| `ops` | `fabric/src/dasein/ops.rs` | Dasein operations |
| `types` | `fabric/src/dasein/types.rs` | Self-field type definitions |

### policy/ — Execution policy

| Module | File | Purpose |
|--------|------|---------|
| `execpolicy` | `fabric/src/policy/execpolicy.rs` | Execution policy definitions |
| `permission_authority` | `fabric/src/policy/permission_authority.rs` | Permission authority delegation |
| `verifier` | `fabric/src/policy/verifier.rs` | Policy verification |

### primitives/ — Foundational abstractions

| Module | File | Purpose |
|--------|------|---------|
| `cognitive` | `fabric/src/primitives/cognitive.rs` | Cognitive primitive types (Command/Query/Event/Stream) |
| `comm` | `fabric/src/primitives/comm.rs` | Communication primitives |

### kernel/ — Fabric kernel services

| Module | File | Purpose |
|--------|------|---------|
| `debug` | `fabric/src/kernel/debug.rs` | Debug facilities |
| `debug_bus` | `fabric/src/kernel/debug_bus.rs` | Bus debugging |
| `error` | `fabric/src/kernel/error.rs` | `AgentError`, `ErrorSeverity`, `ErrorCategory`, `BackoffStrategy`, `DegradationStrategy` |
| `observable` | `fabric/src/kernel/observable.rs` | Observability hooks |
| `registry` | `fabric/src/kernel/registry.rs` | Component registry |

### contract/ — Cross-crate contract definitions

| Module | File | Purpose |
|--------|------|---------|
| `contract` | `fabric/src/contract/mod.rs` | Cross-crate integration contracts |

---

## Key Design Principles

1. **Zero implementations** — This crate only defines contracts. All concrete logic lives in `executive`, `corpus`, `cognit`, `dasein`, `metacog`, `mnemosyne`, `fabric`.

2. **Two PermissionLevel types** — `capability::PermissionLevel` (ReadOnly/SandboxWrite/...) for subsystem capabilities, and `tool::PermissionLevel` (L0-L3) for tool access control. The tool variant is re-exported as `ToolPermissionLevel` to avoid name conflicts.

3. **Subsystem trait** — Every major subsystem implements `Subsystem` (name, version, init, shutdown, health). Higher-level traits (`CognitOps`, `BodyRuntime`, `MetaRuntimeOps`, `SelfFieldOps`) extend this base.

4. **CommunicationBus** — The single event bus system lives at `fabric/src/ipc/bus/communication_bus.rs`. It replaces the earlier `EventBus` trait and `KernelEventBus` wrapper.

---

## Related Docs

- [abi/types.md](types.md) — Shared types, traits, and inter-module interfaces (merged from shared/)
