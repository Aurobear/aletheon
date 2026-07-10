# ABI Crate — Zero-Implementation Trait Definitions

> Code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

**Crate:** `base`
**Purpose:** Core trait definitions and shared types for the Aletheon persistent self-evolving runtime. Contains **zero implementations** — only interfaces, like Linux kernel header files defining the contract between subsystems.

---

## Module Index

| Module | File | Purpose |
|--------|------|---------|
| `subsystem` | `base/src/subsystem.rs` | `Subsystem` trait, `SubsystemHealth`, `SubsystemContext`, `Version` |
| `event` | `base/src/event.rs` | `Event`, `EventType`, `Priority`, `SubscriptionId`, `EventHandler` |
| `event_bus` | `base/src/event_bus.rs` | `EventBus` trait for inter-subsystem event routing |
| `context` | `base/src/context.rs` | `Context`, `TraceState` |
| `capability` | `base/src/capability.rs` | `Capability`, `CapabilitySet`, `PermissionLevel` |
| `body` | `base/src/body.rs` | `Action`, `ActionResult`, `BodyRuntime` trait |
| `memory` | `base/src/memory.rs` | `MemoryBackend`, `MemoryEntry`, `MemoryHandle`, `MemoryQuery`, `MemoryType`, `MemoryFilter`, `CompactStrategy`, `CompactResult`, `MemoryStats` |
| `self_field` | `base/src/self_field.rs` | `SelfFieldOps` trait, `Verdict`, `Intent`, `Identity`, `Care`, `Conflict`, `Resolution`, `MutationIntent` |
| `brain` | `base/src/brain.rs` | `BrainCoreOps` trait, `Plan`, `PlanStep`, `CostEstimate`, `ExecutionResult`, `Reflection`, `Critique`, `LearnedRule`, `Experience`, `Observation` |
| `meta` | `base/src/meta.rs` | `MetaRuntimeOps` trait, `RuntimeCandidate`, `TestResult`, `Evaluation`, `MigrationResult` |
| `runtime` | `base/src/runtime.rs` | `RuntimeOps` trait, `AgentInfo`, `AgentStatus`, `ScheduledTask`, `ScheduleKind`, `StepResult` |
| `genome` | `base/src/genome.rs` | `Genome` model (topology, identity, boundary, care, memory, mutation, lifecycle) |
| `paths` | `base/src/paths.rs` | Standard filesystem paths |
| **Shared types** | | |
| `message` | `base/src/message.rs` | `Message`, `ContentBlock`, `Role`, `ImageSource`, `ToolCall` |
| `tool` | `base/src/tool.rs` | `Tool` trait, `ToolResult`, `ToolResultMeta`, `ToolContext`, `PermissionLevel` (L0-L3) |
| `sandbox` | `base/src/sandbox.rs` | `SandboxBackend`, `SandboxConfig`, `SandboxResult`, `SandboxCapabilities`, `IsolationLevel` |
| `ipc_types` | `base/src/ipc_types.rs` | `IpcBackend`, `IpcPreference`, `IpcProbeError`, `AgentMessage`, `AgentId`, `MessageType`, `IpcPriority` |
| `llm_types` | `base/src/llm_types.rs` | `ToolDefinition` |
| `error` | `base/src/error.rs` | `AgentError`, `ErrorSeverity`, `ErrorCategory`, `BackoffStrategy`, `DegradationStrategy` |

---

## Key Design Principles

1. **Zero implementations** — This crate only defines contracts. All concrete logic lives in `runtime`, `corpus`, `cognit`, `dasein`, `metacog`, `memory`, `base`.

2. **Two PermissionLevel types** — `capability::PermissionLevel` (ReadOnly/SandboxWrite/...) for subsystem capabilities, and `tool::PermissionLevel` (L0-L3) for tool access control. The tool variant is re-exported as `ToolPermissionLevel` to avoid name conflicts.

3. **Subsystem trait** — Every major subsystem implements `Subsystem` (name, version, init, shutdown, health). Higher-level traits (`BrainCoreOps`, `BodyRuntime`, `MetaRuntimeOps`, `SelfFieldOps`) extend this base.

---

## Related Docs

- [abi/types.md](types.md) — Shared types, traits, and inter-module interfaces (merged from shared/)
