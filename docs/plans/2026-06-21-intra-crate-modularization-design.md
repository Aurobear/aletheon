# Intra-Crate Modularization Design

**Date:** 2026-06-21
**Status:** Approved
**Scope:** All crates — internal module restructuring
**Motivation:** After Phases 1-4 (crate rename, comm merge, body split, communication architecture), the INTERNAL structure of several crates still has flat files with no logical grouping. This phase modularizes within each crate.

## Problem Statement

After the inter-crate architectural redesign, several crates have poor internal organization:

| Crate | Problem | Severity |
|-------|---------|----------|
| `base/src/` | 35 flat .rs files, no subdirectories (except merged `comm/`) | Critical |
| `memory/src/` | 10 flat .rs files, no logical grouping | Medium |
| `cognit/src/core/mod.rs` | 1350 lines monolithic file | Critical |
| `interact/src/ui/mod.rs` | 2388 lines monolithic TUI event loop | Critical |
| `runtime` | handler.rs (2467), fact_store.rs (1359), react_loop.rs (1355) | High |

Additionally, the `comm/` subdirectory (merged from aletheon-comm in Phase 2) needs to be fully dissolved into base's module structure, not kept as a separate subtree.

## Design Principles

1. **Linux kernel directory naming** — `abi/` (like `include/`), `types/`, `ipc/` (like `net/`), `kernel/`, `events/`
2. **Logical domain grouping** — files that belong together live together
3. **API compatibility** — `lib.rs` re-exports preserve the existing public API surface
4. **No behavioral changes** — this is pure restructuring, no logic changes
5. **comm fully dissolved** — the `comm/` directory disappears completely; contents fuse into `ipc/` and `kernel/`

## Phase 5A: base/src/ Modularization

### New Directory Structure

```
base/src/
├── abi/                        ← Subsystem trait contracts (like kernel include/)
│   ├── mod.rs
│   ├── subsystem.rs            (108 lines) — Subsystem lifecycle trait
│   ├── body.rs                 (90 lines)  — BodyRuntime (execution HAL)
│   ├── brain.rs                (290 lines) — BrainCoreOps (cognitive scheduler)
│   ├── self_field.rs           (314 lines) — SelfFieldOps (policy engine / LSM)
│   ├── memory.rs               (159 lines) — MemoryBackend (VFS-like abstraction)
│   ├── meta.rs                 (89 lines)  — MetaRuntimeOps (self-modification)
│   ├── runtime.rs              (66 lines)  — RuntimeOps (orchestration)
│   └── event_bus.rs            (77 lines)  — EventBus trait (deprecated)
│
├── types/                      ← Shared data types
│   ├── mod.rs
│   ├── agent.rs                (32 lines)  — Pid
│   ├── context.rs              (88 lines)  — Context, TraceState
│   ├── capability.rs           (107 lines) — Capability, PermissionLevel, CapabilitySet
│   ├── genome.rs               (101 lines) — Genome, Topology, *Spec types
│   ├── message.rs              (187 lines) — ContentBlock, Message, Role
│   ├── tool.rs                 (202 lines) — Tool trait, ToolContext, ToolResult
│   ├── llm_types.rs            (13 lines)  — ToolDefinition
│   ├── sandbox.rs              (88 lines)  — IsolationLevel, SandboxBackend
│   ├── hook.rs                 (128 lines) — HookPoint, HookContext, HookResult
│   ├── hook_ext.rs             (51 lines)  — HookConfig, HookType
│   ├── permission.rs           (211 lines) — PermissionMode, PermissionContext
│   ├── resource.rs             (180 lines) — ManagedResource, ResourceState
│   └── paths.rs                (71 lines)  — Path constants
│
├── events/                     ← Event system (types + infrastructure)
│   ├── mod.rs
│   ├── event.rs                (189 lines) — EventType, Priority, Event trait
│   ├── evolution.rs            (126 lines) — Self-evolution event types
│   ├── ui_event.rs             (211 lines) — UiEvent, CollaborationMode
│   ├── subscription.rs         (180 lines) ← from comm/impl/
│   ├── routing_policy.rs       (79 lines)  ← from comm/impl/
│   ├── event_log.rs            (149 lines) ← from comm/impl/
│   └── event_bridge.rs         (145 lines) ← from comm/bridge/
│
├── ipc/                        ← Inter-process communication (like net/)
│   ├── mod.rs
│   ├── envelope.rs             (239 lines) — Envelope, Endpoint, Target, Pattern, Payload
│   ├── protocol.rs             (15 lines)  — Protocol trait
│   ├── transport.rs            (51 lines)  — Transport trait, TransportKind
│   ├── ipc.rs                  (129 lines) — IpcMessage, Signal, ForkDirective
│   ├── ipc_types.rs            (126 lines) — AgentMessage, IpcBackend trait
│   ├── bus/                    ← Communication bus implementations
│   │   ├── mod.rs
│   │   ├── communication_bus.rs    (255 lines) ← from comm/impl/
│   │   ├── kernel_bus.rs           (150 lines) ← from comm/impl/
│   │   ├── in_process.rs           (443 lines) ← from comm/impl/
│   │   ├── pubsub.rs               (41 lines)  ← from comm/impl/
│   │   └── request_response.rs     (99 lines)  ← from comm/impl/
│   ├── transport/              ← Transport implementations
│   │   ├── mod.rs
│   │   └── unix_socket.rs          (357 lines) ← from comm/impl/
│   └── backends/               ← IpcBackend implementations
│       ├── mod.rs
│       ├── io_uring.rs         (280 lines) ← from comm/impl/ipc/
│       ├── json_rpc.rs         (93 lines)  ← from comm/impl/ipc/
│       ├── manager.rs          (337 lines) ← from comm/impl/ipc/
│       ├── priority_queue.rs   (180 lines) ← from comm/impl/ipc/
│       ├── shared_mem.rs       (262 lines) ← from comm/impl/ipc/
│       ├── transport_adapter.rs (104 lines) ← from comm/impl/ipc/
│       └── unix_socket.rs      (338 lines) ← from comm/impl/ipc/
│
├── kernel/                     ← Core infrastructure
│   ├── mod.rs
│   ├── observable.rs           (28 lines)  — Observable trait, SubsystemStatus
│   ├── registry.rs             (36 lines)  — Registry trait, RegistrationId
│   ├── debug.rs                (80 lines)  — DebugLevel, Tracepoint, DebugSink
│   ├── debug_bus.rs            (521 lines) ← from comm/impl/ (DebugBusHook, EventRecorder, PerfCounter)
│   └── error/                  ← Error handling
│       ├── mod.rs
│       ├── types.rs            — ErrorSeverity, ErrorCategory, *ErrorKind enums
│       ├── agent_error.rs      — AgentError struct + impls
│       ├── backoff.rs          — BackoffStrategy, DegradationStrategy, DegradationChain
│       └── tool_handler.rs     — handle_tool_error, ToolErrorAction
│
├── policy/                     ← Execution policy engine
│   ├── mod.rs
│   └── execpolicy.rs           (299 lines) — Policy, PrefixRule, NetworkRule
│
├── dasein/                     ← Phenomenological module (unchanged)
│   └── dasein.rs               (452 lines)
│
└── lib.rs                      ← Re-exports preserving API compatibility
```

### comm/ Dissolution Map

The `comm/` directory is completely dissolved. Each file maps to its new home:

| comm/ File | New Location | Rationale |
|------------|-------------|-----------|
| `comm/bridge/event_bridge.rs` | `events/event_bridge.rs` | Bridges Event↔Envelope, lives with event system |
| `comm/impl/subscription.rs` | `events/subscription.rs` | EventHandler subscription management |
| `comm/impl/routing_policy.rs` | `events/routing_policy.rs` | EventType+Priority routing logic |
| `comm/impl/event_log.rs` | `events/event_log.rs` | Event ring buffer |
| `comm/impl/pubsub.rs` | `ipc/bus/pubsub.rs` | Implements Protocol trait |
| `comm/impl/request_response.rs` | `ipc/bus/request_response.rs` | Implements Protocol trait |
| `comm/impl/communication_bus.rs` | `ipc/bus/communication_bus.rs` | Unified bus entry point |
| `comm/impl/kernel_bus.rs` | `ipc/bus/kernel_bus.rs` | EventBus implementation |
| `comm/impl/in_process.rs` | `ipc/bus/in_process.rs` | Primary Transport implementation |
| `comm/impl/unix_socket_transport.rs` | `ipc/transport/unix_socket.rs` | Transport implementation |
| `comm/impl/debug_bus.rs` | `kernel/debug_bus.rs` | Debug infrastructure (depends only on crate::debug) |
| `comm/impl/ipc/*` (7 files) | `ipc/backends/*` | IpcBackend implementations |

### Dependency Analysis Summary

- **No circular dependencies** between comm files and base modules
- **Two independent clusters** in comm: Envelope/Transport (Cluster A) and IpcBackend (Cluster B), zero cross-dependency
- **Only 3 types consumed outside base:** `CommunicationBus`, `DebugBusHook`/`EventFilter`/`PerfCounter`
- **All leaf files** can move independently; hub files (communication_bus, kernel_bus, in_process) move last

### API Compatibility

`lib.rs` re-exports are updated to point to new paths. External consumers using `base::CommunicationBus`, `base::KernelEventBus`, etc. continue to work unchanged. The `pub mod comm;` declaration is removed; replaced by the new module tree.

## Phase 5B: memory/src/ Modularization

```
memory/src/
├── types/              ← Data types
│   ├── mod.rs
│   ├── entry.rs        — MemoryEntry, MemoryHandle
│   ├── query.rs        — MemoryQuery, MemoryFilter
│   └── stats.rs        — MemoryStats, CompactStrategy, CompactResult
├── backends/           ← Storage backends
│   ├── mod.rs
│   ├── episodic.rs     (1179 lines — future split candidate)
│   ├── semantic.rs     (1115 lines — future split candidate)
│   ├── procedural.rs
│   └── self_memory.rs
├── ops/                ← Operations
│   ├── mod.rs
│   ├── router.rs       (845 lines — future split candidate)
│   ├── consolidation.rs
│   ├── decay.rs
│   ├── activation.rs
│   └── schema.rs
├── testing/
│   ├── mod.rs
│   └── mock_memory.rs
└── lib.rs
```

## Phase 5C: cognit/src/core/mod.rs Split

The 1350-line `core/mod.rs` is split into focused files:

| Section | Lines | New File |
|---------|-------|----------|
| ExperienceSummarizer | ~138 | `core/experience_summarizer.rs` |
| BrainCoreConfig | ~19 | stays in `core/mod.rs` |
| BrainCore struct + builder | ~361 | stays in `core/mod.rs` |
| Subsystem impl | ~32 | `core/brain_core_subsystem.rs` |
| BrainCoreOps impl | ~207 | `core/brain_core_ops.rs` |
| Tests | ~543 | `core/tests.rs` |

After split, `core/mod.rs` is ~400 lines (struct + builder + re-exports).

## Phase 5D: interact/src/ui/mod.rs Split

The 2388-line `ui/mod.rs` is split into focused modules:

```
interact/src/ui/
├── mod.rs              ← App struct + run() + re-exports (~400 lines)
├── app/
│   ├── mod.rs
│   ├── lifecycle.rs    — run_app, initialization, cleanup
│   ├── key_handler.rs  — handle_key, handle_mouse
│   └── submit.rs       — submit_message, send_to_daemon
├── render/
│   ├── mod.rs
│   ├── draw.rs         — draw_with_recorder, layout
│   ├── header.rs       — render_header
│   └── input.rs        — render_input
├── response.rs         — handle_event, process_response, format_* helpers
├── test_infra.rs       — TestConfig, FrameSnapshot, FrameRecorder, EventRecorder
├── [all existing ui/*.rs files unchanged]
└── lib.rs
```

## Phase 5E: Runtime Large File Split

Individual file splits (directory structure unchanged):

**handler.rs (2467 lines):**
```
impl/daemon/
├── handler/
│   ├── mod.rs          — re-exports
│   ├── chat.rs         — chat message handling
│   ├── rpc.rs          — JSON-RPC dispatch
│   └── format.rs       — format_* helper functions
```

**fact_store.rs (1359 lines):**
```
impl/memory/
├── fact_store/
│   ├── mod.rs          — re-exports
│   ├── index.rs        — indexing and storage
│   └── query.rs        — query and search
```

**react_loop.rs (1355 lines):**
```
core/
├── react_loop/
│   ├── mod.rs          — re-exports
│   ├── step.rs         — step execution
│   └── tool_exec.rs    — tool execution logic
```

## Implementation Order

1. **Phase 5A** — base/src/ modularization + comm/ dissolution (largest, most impactful)
2. **Phase 5B** — memory/src/ modularization
3. **Phase 5C** — cognit/src/core/mod.rs split
4. **Phase 5D** — interact/src/ui/mod.rs split
5. **Phase 5E** — runtime large file splits

Each phase is independently compilable and testable. `cargo check` after each phase.

## Migration Strategy

1. Create new directory structure
2. Move files using `git mv` (preserves history)
3. Update `mod.rs` declarations
4. Update internal `use` paths (`crate::comm::*` → `crate::ipc::*`, etc.)
5. Update `lib.rs` re-exports
6. Update external consumers (2 runtime files use deep `base::comm::r#impl::debug_bus::` paths)
7. `cargo check` + `cargo test`

## Validation

- `cargo check` passes after each phase
- `cargo test` passes after each phase
- All existing `use base::*` imports continue to work (re-export compatibility)
- No behavioral changes — pure restructuring
