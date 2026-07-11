# Fabric Crate — Shared Contracts and Communication

> Code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `fabric`
**Purpose:** Core trait definitions and shared types for the Aletheon persistent self-evolving runtime. Contains **zero implementations** — only interfaces, like Linux kernel header files defining the contract between subsystems.

---

## Module Index

| Module | File | Purpose |
|--------|------|---------|
| `subsystem` | `fabric/src/subsystem.rs` | `Subsystem` trait, `SubsystemHealth`, `SubsystemContext`, `Version` |
| `event` | `fabric/src/event.rs` | `Event`, `EventType`, `Priority`, `SubscriptionId`, `EventHandler` |
| `event_bus` | `fabric/src/event_bus.rs` | `EventBus` trait for inter-subsystem event routing |
| `context` | `fabric/src/context.rs` | `Context`, `TraceState` |
| `capability` | `fabric/src/capability.rs` | `Capability`, `CapabilitySet`, `PermissionLevel` |
| `body` | `fabric/src/body.rs` | `Action`, `ActionResult`, `BodyRuntime` trait |
| `memory` | `fabric/src/memory.rs` | `MemoryBackend`, `MemoryEntry`, `MemoryHandle`, `MemoryQuery`, `MemoryType`, `MemoryFilter`, `CompactStrategy`, `CompactResult`, `MemoryStats` |
| `self_field` | `fabric/src/self_field.rs` | `SelfFieldOps` trait, `Verdict`, `Intent`, `Identity`, `Care`, `Conflict`, `Resolution`, `MutationIntent` |
| `cognit` | `fabric/src/include/cognit.rs` | `CognitOps` trait, `Plan`, `PlanStep`, `CostEstimate`, `ExecutionResult`, `Reflection`, `Critique`, `LearnedRule`, `Experience`, `Observation` |
| `meta` | `fabric/src/meta.rs` | `MetaRuntimeOps` trait, `RuntimeCandidate`, `TestResult`, `Evaluation`, `MigrationResult` |
| `runtime` | `fabric/src/runtime.rs` | `RuntimeOps` trait, `AgentInfo`, `AgentStatus`, `ScheduledTask`, `ScheduleKind`, `StepResult` |
| `genome` | `fabric/src/genome.rs` | `Genome` model (topology, identity, boundary, care, memory, mutation, lifecycle) |
| `paths` | `fabric/src/paths.rs` | Standard filesystem paths |
| **Shared types** | | |
| `message` | `fabric/src/message.rs` | `Message`, `ContentBlock`, `Role`, `ImageSource`, `ToolCall` |
| `tool` | `fabric/src/tool.rs` | `Tool` trait, `ToolResult`, `ToolResultMeta`, `ToolContext`, `PermissionLevel` (L0-L3) |
| `sandbox` | `fabric/src/sandbox.rs` | `SandboxBackend`, `SandboxConfig`, `SandboxResult`, `SandboxCapabilities`, `IsolationLevel` |
| `ipc_types` | `fabric/src/ipc_types.rs` | `IpcBackend`, `IpcPreference`, `IpcProbeError`, `AgentMessage`, `AgentId`, `MessageType`, `IpcPriority` |
| `llm_types` | `fabric/src/llm_types.rs` | `ToolDefinition` |
| `error` | `fabric/src/error.rs` | `AgentError`, `ErrorSeverity`, `ErrorCategory`, `BackoffStrategy`, `DegradationStrategy` |

---

## Key Design Principles

1. **Zero implementations** — This crate only defines contracts. All concrete logic lives in `executive`, `corpus`, `cognit`, `dasein`, `metacog`, `mnemosyne`, `fabric`.

2. **Two PermissionLevel types** — `capability::PermissionLevel` (ReadOnly/SandboxWrite/...) for subsystem capabilities, and `tool::PermissionLevel` (L0-L3) for tool access control. The tool variant is re-exported as `ToolPermissionLevel` to avoid name conflicts.

3. **Subsystem trait** — Every major subsystem implements `Subsystem` (name, version, init, shutdown, health). Higher-level traits (`CognitOps`, `BodyRuntime`, `MetaRuntimeOps`, `SelfFieldOps`) extend this base.

---

## Related Docs

- [abi/types.md](types.md) — Shared types, traits, and inter-module interfaces (merged from shared/)
