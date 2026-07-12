# Phase 3-6 Production Wiring — Design

> Status update after PR #54 merge. Most Phase 3-6 infrastructure is already
> in production; the remaining work is targeted gap-closing, not new construction.

## 1. Current State (after PR #54 merge)

### Already wired to production

| What | Where | How |
|------|-------|-----|
| **ContextSpace** per-turn creation | `executive/src/service/daemon_turn/execute.rs:287-328` | `SpaceId::new()` + `attach_region` (Session, Agora) + `set_overlay(turn_input)` |
| **Agora propose→commit** for tool evidence | `execute.rs:511-525` | On `Event::ToolResult`, creates `AgoraOperation::AcceptEvidence`, calls `agora.propose()` then `agora.commit()`, increments `agora_version` |
| **Admission gate** before tool execution | `execute.rs:344-413` | `adm.admit(AdmissionRequest)` → `ExecutionPermit` → `exec.execute(&permit)` → `adm.settle(UsageReport)` |
| **OperationScope** cancellation | `execute.rs:84-94` | Per-turn `OperationScope` with `CancellationToken` checked before admission |
| **ProcessTable/OperationTable** registration | `execute.rs:38-65` | Main agent registered in process table, turn operation submitted |
| **SandboxFirst** fail-closed | `execute.rs:123-127` | SelfField `SandboxFirst` → `SandboxRequirement::Required` on admission; `SandboxDecision::Required` → fail |
| **EnvelopeV2/Mailbox/Stream** contracts | `fabric/src/ipc/` | Full type system, tests pass |
| **Kernel crate** (standalone) | `kernel/src/` | ProcessTable, OperationTable, Clock, SupervisorTree, Admission, Budget, Lease, SpaceManager |
| **ServicePorts** bundling | `kernel/src/service_ports.rs` | Bundles kernel services; `CoreSystems.ports` field exists |
| **AgoraWorkspace** transaction model | `agora/src/workspace.rs:57-167` | `propose(CAS)`, `commit`, `reject`, `VersionConflict`, TTL expiry |

### Actually still missing (targeted gaps)

1. **EnvelopeV2 event stream** — TurnService event pumping uses
   `tokio::sync::mpsc::channel::<Event>(64)` at `execute.rs:429`. EnvelopeV2
   exists but isn't the event transport layer yet.
2. **Schema version rejection** — `fabric/src/ipc/envelope_v2.rs` has
   `SchemaId` but no `UnsupportedSchema` enforcement at boundaries.
3. **CoreSystems field grouping** — 30+ individual fields, many are
   already available via `ServicePorts`. Not a functional gap, but
   grouping into named sub-structs (`MemoryGroup`, `SecurityGroup`,
   `CorpusGroup`) improves navigation.
4. **Project rules** — `.claude/` and `.codex/` rules documenting the
   module conventions and architectural invariants for agentic workers.

## 2. Non-Goals

- Eliminating CoreSystems (user said: don't worry about god object,
  just keep responsibilities clear)
- Renaming crates or moving between crates
- Adding new kernel primitives
- Deep refactors of existing logic
- Full EnvelopeV2 replacement of all event paths (just the TurnService one)

## 3. PR Plan

### PR-A: EnvelopeV2 Event Stream

Switch TurnService event pumping from `mpsc::channel<Event>` to
EnvelopeV2 Stream, with schema version enforcement.

**Files:**
- `crates/fabric/src/ipc/stream.rs` — add `TurnEventSchema` and
  `SchemaRejection` type
- `crates/executive/src/service/daemon_turn/execute.rs` — replace
  `mpsc::channel` with `EnvelopeV2` stream; add schema check on recv
- `crates/executive/tests/turn_pipeline_order.rs` — verify event
  ordering survives the transport change

**What it does not do:** replace `Event` trait elsewhere or delete
`legacy_bridge.rs`. Only the TurnService event path changes.

**Acceptance:**
```bash
cargo test -p fabric stream_backpressure
cargo test -p executive turn_pipeline_order
cargo test -p executive turn_service_equivalence
cargo check --workspace --all-targets
```

### PR-B: CoreSystems Field Grouping

Group `CoreSystems` fields into logical sub-structs without changing
ownership or type signatures.

**Target groups (in `crates/executive/src/core/`):**
- `memory_group.rs` — EpisodicMemory, RecallMemory, CoreMemory,
  FactStore, AutoMemory, ObjectiveStore
- `security_group.rs` — ToolRunnerWithGuard, StormBreaker,
  approval_rx, pending_approvals, session_approvals
- `corpus_group.rs` — ToolRegistry, SkillLoader, SkillRouter,
  HookRegistry, hooks_config
- `session_group.rs` — default_session_id, session_created_at,
  cached_prefix, memory_queue, context_window

**CoreSystems becomes:**
```rust
pub struct CoreSystems {
    pub ports: ServicePorts,
    pub runtime: Arc<Mutex<AletheonExecutive>>,
    pub memory: MemoryGroup,
    pub security: SecurityGroup,
    pub corpus: CorpusGroup,
    pub session: SessionGroup,
    // Remainder that doesn't fit groups (OK to keep flat)
    pub self_field: Arc<Mutex<SelfField>>,
    pub reflector: Reflector,
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
    pub debug_handler: Arc<DebugHandler>,
    pub debug_perf: Arc<PerfCounter>,
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    pub config_prompt: String,
    pub data_dir: PathBuf,
}
```

**What it does not do:** move fields into `ServicePorts`, change
`Arc<Mutex<>>` wrappers, change initialization order.

**Acceptance:**
```bash
cargo check --workspace --all-targets
cargo test -p executive
rg "subsystems\.[a-z_]+" crates/executive/src/service/daemon_turn/ | sort | uniq -c
```

### PR-C: Project Rules for Agentic Workers

Write `.claude/settings.json` and `.codex/config.toml` with module
conventions and architectural invariants.

**Rules to encode:**
1. Module structure: each crate uses `src/<domain>/mod.rs` with
   sub-files; no single-file domain dumps
2. Service access: production path goes through `ServicePorts` for
   kernel primitives, `CoreSystems` for domain services
3. Safety invariant: all tool execution must go through
   `AdmissionController::admit()` → `ExecutionPermit`
4. Shared state: writes to Agora must go through `propose()` →
   `commit()`, never direct `publish()`
5. Test discipline: kernel tests use `VirtualClock`, no real `sleep`
6. PR constraints: Phase 3-6 is wiring-only phase; no new kernel
   primitives, no crate renames

**Files:**
- `.claude/settings.json` — permission rules for the workspace
- `.codex/config.toml` — Codex-specific rules
- `.claude/agents/developer.md` — update with crate conventions
- `.codex/agents/developer.md` — update with crate conventions

## 4. Execution Order

```text
PR-A (EnvelopeV2 stream) → PR-B (CoreSystems grouping) → PR-C (project rules)
```

PR-A and PR-B can run in parallel (they touch different subsystems).
PR-C depends on neither (pure config/docs).

## 5. Verification Gate

```bash
# Full gate for the wiring phase
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo doc --workspace --no-deps

# Specific invariants
rg "risk_level|RiskLevel::" crates/executive/src/service/daemon_turn/execute.rs
rg "agora\.propose|agora\.commit" crates/executive/src/service/daemon_turn/execute.rs
rg "adm\.admit|admission\.admit" crates/executive/src/service/daemon_turn/execute.rs
```

## 6. Non-Targets (explicitly excluded)

- CRDT / distributed consistency
- Full legacy Event cleanup
- Any kernel primitive additions
- Crate renames or cross-crate moves
- Performance benchmarks (not regressing is sufficient)
