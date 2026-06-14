# ABI Crate — Zero-Implementation Trait Definitions

> Code paths updated to aletheon-* crate structure

**Crate:** `aletheon-abi`
**Purpose:** Core trait definitions and shared types for the Aletheon persistent self-evolving runtime. Contains **zero implementations** — only interfaces, like Linux kernel header files defining the contract between subsystems.

---

## Module Index

| Module | File | Purpose |
|--------|------|---------|
| `subsystem` | `aletheon-abi/src/subsystem.rs` | `Subsystem` trait, `SubsystemHealth`, `SubsystemContext`, `Version` |
| `event` | `aletheon-abi/src/event.rs` | `Event`, `EventType`, `Priority`, `SubscriptionId`, `EventHandler` |
| `event_bus` | `aletheon-abi/src/event_bus.rs` | `EventBus` trait for inter-subsystem event routing |
| `context` | `aletheon-abi/src/context.rs` | `Context`, `TraceState` |
| `capability` | `aletheon-abi/src/capability.rs` | `Capability`, `CapabilitySet`, `PermissionLevel` |
| `body` | `aletheon-abi/src/body.rs` | `Action`, `ActionResult`, `BodyRuntime` trait |
| `memory` | `aletheon-abi/src/memory.rs` | `MemoryBackend`, `MemoryEntry`, `MemoryHandle`, `MemoryQuery`, `MemoryType`, `MemoryFilter`, `CompactStrategy`, `CompactResult`, `MemoryStats` |
| `self_field` | `aletheon-abi/src/self_field.rs` | `SelfFieldOps` trait, `Verdict`, `Intent`, `Identity`, `Care`, `Conflict`, `Resolution`, `MutationIntent` |
| `brain` | `aletheon-abi/src/brain.rs` | `BrainCoreOps` trait, `Plan`, `PlanStep`, `CostEstimate`, `ExecutionResult`, `Reflection`, `Critique`, `LearnedRule`, `Experience`, `Observation` |
| `meta` | `aletheon-abi/src/meta.rs` | `MetaRuntimeOps` trait, `RuntimeCandidate`, `TestResult`, `Evaluation`, `MigrationResult` |
| `runtime` | `aletheon-abi/src/runtime.rs` | `RuntimeOps` trait, `AgentInfo`, `AgentStatus`, `ScheduledTask`, `ScheduleKind`, `StepResult` |
| `genome` | `aletheon-abi/src/genome.rs` | `Genome` model (topology, identity, boundary, care, memory, mutation, lifecycle) |
| `paths` | `aletheon-abi/src/paths.rs` | Standard filesystem paths |
| **Shared types** | | |
| `message` | `aletheon-abi/src/message.rs` | `Message`, `ContentBlock`, `Role`, `ImageSource`, `ToolCall` |
| `tool` | `aletheon-abi/src/tool.rs` | `Tool` trait, `ToolResult`, `ToolResultMeta`, `ToolContext`, `PermissionLevel` (L0-L3) |
| `sandbox` | `aletheon-abi/src/sandbox.rs` | `SandboxBackend`, `SandboxConfig`, `SandboxResult`, `SandboxCapabilities`, `IsolationLevel` |
| `ipc_types` | `aletheon-abi/src/ipc_types.rs` | `IpcBackend`, `IpcPreference`, `IpcProbeError`, `AgentMessage`, `AgentId`, `MessageType`, `IpcPriority` |
| `llm_types` | `aletheon-abi/src/llm_types.rs` | `ToolDefinition` |
| `error` | `aletheon-abi/src/error.rs` | `AgentError`, `ErrorSeverity`, `ErrorCategory`, `BackoffStrategy`, `DegradationStrategy` |

---

## Key Design Principles

1. **Zero implementations** — This crate only defines contracts. All concrete logic lives in `aletheon-runtime`, `aletheon-body`, `aletheon-brain`, `aletheon-self`, `aletheon-meta`, `aletheon-memory`, `aletheon-comm`.

2. **Two PermissionLevel types** — `capability::PermissionLevel` (ReadOnly/SandboxWrite/...) for subsystem capabilities, and `tool::PermissionLevel` (L0-L3) for tool access control. The tool variant is re-exported as `ToolPermissionLevel` to avoid name conflicts.

3. **Subsystem trait** — Every major subsystem implements `Subsystem` (name, version, init, shutdown, health). Higher-level traits (`BrainCoreOps`, `BodyRuntime`, `MetaRuntimeOps`, `SelfFieldOps`) extend this base.

---

## Related Docs

- [abi/types.md](types.md) — Shared types, traits, and inter-module interfaces (merged from shared/)
