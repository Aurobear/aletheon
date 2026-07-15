# X01 Executive Use-Case Ports Implementation Plan

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove concrete domain stores, registries, and mutexes from JSON-RPC request code and split turn context/projection behavior into bounded Executive use-case services.

**Architecture:** Keep `CoreSystems` temporarily as the private bootstrap input for X02, but never pass it to request-facing handlers. Bootstrap constructs a private `HandlerPorts` bundle of narrow async traits. Concrete adapters own existing store/registry locks; handlers parse JSON-RPC, call one use-case method, and format the response. `TurnPipeline` delegates context assembly and post-turn projection to extracted services while retaining synchronous approval and capability transactions.

**Tech Stack:** Rust, Tokio, async-trait, existing Fabric contracts, existing Mnemosyne/Goal/Approval stores, architecture fitness tests.

**Prerequisites:** S02 and K02 complete. Evidence: `docs/plans/2026-07-16-original-plan-coverage-matrix.md:28-32`.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:296-372`, `:1034-1063`.

**Current-code anchors (2026-07-16):** request handlers hold `Arc<CoreSystems>` at `crates/executive/src/impl/daemon/handler/mod.rs:40-45`; memory RPC reaches `FactStore` locks at `crates/executive/src/impl/daemon/handler/rpc/rpc_memory.rs:21-23`; goal RPC reaches `ObjectiveStore` locks at `crates/executive/src/impl/daemon/handler/rpc/rpc_goal.rs:28-30`; approval RPC reaches the repository at `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs:44-45`; `TurnPipeline` reaches concrete memory, Corpus, security, SelfField, and runtime fields at `crates/executive/src/service/turn_pipeline.rs:154-488`; bootstrap constructs the complete bundle in `crates/executive/src/impl/daemon/handler/init.rs:1029-1100`.

**Invariants:**

- Handler modules may depend on `HandlerPorts`, protocol values, and handler-local connection/session routing state, never `CoreSystems`, `MemoryGroup`, `CorpusGroup`, `SecurityGroup`, a domain store, or `Arc<Mutex<ConcreteDomain>>`.
- Use-case traits expose typed inputs/results and sanitized application errors; they do not expose locks or database handles.
- Approval and capability execution remain synchronous and explicit.
- X01 does not delete `CoreSystems` or split bootstrap; X02 owns that deletion gate.
- Existing JSON-RPC method names, result shapes, error codes, and daemon/exec turn behavior remain compatible.

---

### Task 1: Add a failing request-boundary architecture test

**Files:**
- Create: `crates/executive/tests/request_use_case_boundaries.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Add a source test that scans `crates/executive/src/impl/daemon/handler/rpc/*.rs` and rejects `subsystems`, `CoreSystems`, `MemoryGroup`, `CorpusGroup`, `SecurityGroup`, `FactStore`, `ObjectiveStore`, `ApprovalRepository`, and direct `.lock()` calls.
- [ ] Add a second assertion that `RequestHandler` has a `ports: Arc<HandlerPorts>` field and no `subsystems` field.
- [ ] Run `cargo test -p executive --test request_use_case_boundaries`; expected failure identifies current concrete accesses.
- [ ] Add the same production scan to `scripts/architecture-check.sh` so the boundary cannot regress after the test passes.

### Task 2: Introduce typed fact-memory use cases

**Files:**
- Create: `crates/mnemosyne/src/fact_service.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_memory.rs`
- Test: `crates/executive/tests/fact_service.rs`

- [x] Define `FactUseCases` with async `add`, `list`, `search`, `show`, `forget`, and `set_pinned` methods. Define exact request structs for add/list/search and return the stable request-facing `FactView`, IDs, booleans, or `FactServiceError::{NotFound, InvalidInput, Store}`.
- [x] Implement `DefaultFactUseCases` inside Mnemosyne with one private `Arc<tokio::sync::Mutex<FactStore>>`; preserve the existing governed defaults (`general`, trust `0.7`, semantic tier, search trust `0.15`, list `50`, search `20`).
- [x] Test duplicate add, scoped list/search, show-not-found, archive/hard-delete, pin/unpin, and that errors never expose a database handle.
- [x] Change memory RPC handlers to parse parameters, call `self.ports.facts`, and preserve existing JSON-RPC result/error shapes.
- [x] Run `cargo test -p executive --test fact_service`, `cargo check -p executive --all-targets`, and `cargo clippy -p executive --all-targets -- -D warnings`; PASS.

**Task 2 evidence:** authoritative port, stable DTO, and private adapter at `crates/mnemosyne/src/fact_service.rs`; request-only handler calls at `crates/executive/src/impl/daemon/handler/rpc/rpc_memory.rs:26-165`; behavioral and static boundary tests at `crates/executive/tests/fact_service.rs`.

### Task 3: Introduce typed goal use cases

**Files:**
- Create: `crates/executive/src/service/goal_service.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_goal.rs`
- Test: `crates/executive/tests/goal_service.rs`

- [x] Define a `GoalUseCases` async trait covering the four legacy objective operations and versioned create/list/pause/run/cancel operations with typed `GoalServiceError::{NotFound, InvalidTransition, Conflict, Store}`.
- [x] Implement `GoalService` with the private existing `Arc<Mutex<ObjectiveStore>>`; keep optimistic version checks and immutable original intent.
- [x] Test create/list/get/resume and every legal/illegal pause/run/cancel transition, including stale-version conflict and terminal cancellation rejection.
- [x] Migrate goal RPC handlers to the port without changing method names, response fields, or error-code mapping.
- [x] Run `cargo test -p executive --test goal_service --test goal_rpc --test goal_lifecycle`; PASS (19 focused tests).

**Task 3 evidence:** request-safe trait/error boundary and private store adapter at `crates/executive/src/service/goal_service.rs:31-234`; bootstrap-only adapter construction at `crates/executive/src/impl/daemon/handler/init.rs:1029-1032`; port-only request handling at `crates/executive/src/impl/daemon/handler/rpc/rpc_goal.rs`; compatibility, legal/illegal transition, terminal, and stale-version coverage at `crates/executive/tests/goal_service.rs:28-165`. `cargo check -p executive --all-targets`, focused Clippy, and architecture fitness also pass; the `rpc_goal` service-locator baseline entry was deleted.

### Task 4: Introduce approval and admin use cases

**Files:**
- Create: `crates/executive/src/service/approval_service.rs`
- Create: `crates/executive/src/service/admin_service.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_admin.rs`
- Test: `crates/executive/tests/handler_use_case_ports.rs`

- [x] Define approval list/show/approve/reject requests and results that carry approval IDs, bounded summaries, states, and version/hash evidence but never repository handles.
- [x] Implement `ApprovalService` over `ApprovalRepository`, the optional `ApplyCoordinator`, and injected `Clock`; preserve owner/hash/expiry/replay checks.
- [x] Define narrow admin operations for skill reload/status, mode/model selection, tool descriptors, hooks, and sub-agent summaries; implementations own the concrete loader/registry/runtime locks.
- [x] Migrate approval/admin RPC code to the traits and test forged/replayed approval, missing optional apply coordinator, reload failure, and bounded list results.

**Task 4 evidence:** typed approval context/resolution/error contracts and the repository/apply/clock adapter are at `crates/executive/src/service/approval_service.rs:17-171`; the pending list is capped at 100 while `ApprovalSnapshot` retains state, version, and subject-hash evidence. Narrow admin operations, fallible skill port, type-erased tool/hook catalogs, lifecycle control, and 200-item caps are at `crates/executive/src/service/admin_service.rs:20-287`. Forged owner, idempotent replay, stale version, expiry, missing apply runtime, and bounded approval coverage is at `crates/executive/tests/approval_service.rs:95-215`; reload success/failure, mode/model, transient approval, shutdown, and request-boundary coverage is at `crates/executive/tests/admin_service.rs:51-169`. Focused tests (19 approval/apply/channel tests and 5 admin tests), focused Clippy, and architecture fitness pass; all three former approval/admin handler service-locator exceptions were deleted.

### Task 5: Extract session/context use cases

**Files:**
- Create: `crates/executive/src/service/context_assembler.rs`
- Create: `crates/executive/src/service/legacy_session_service.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/service/daemon_turn/injection.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_session.rs`
- Modify: `crates/executive/src/impl/daemon/handler/session_routing.rs`
- Test: `crates/executive/tests/context_assembler.rs`
- Test: `crates/executive/tests/session_use_case_port.rs`

- [ ] Define `ContextAssembler::assemble(TurnRequest, canonical_history)` returning one bounded `AssembledContext` with system prefix, recall, core memory, facts, skills, Dasein view, and Agora view in deterministic order.
- [ ] Move injection reads/formatting from daemon-turn and `TurnPipeline` behind existing `MemoryService`, `TurnServices`, and a skill-catalog port; enforce the current item and byte bounds before returning.
- [ ] Define legacy clear/list/resume/compact/create/switch operations behind a `LegacySessionUseCases` trait implemented over `SessionService`, `SessionStore`, and the session registry.
- [ ] Migrate session RPC/routing code to those operations and prove resume/fork/replay/interrupt still use canonical Session/Turn/Item state.
- [ ] Run context/session focused tests plus `session_lifecycle_commands`, `session_append_store`, and `turn_service_equivalence`; expected PASS.

**Task 5 progress:** the request/history boundary, deterministic fragment order (`recall -> core memory -> facts -> skills -> Dasein -> Agora`), UTF-8-safe per-fragment/aggregate caps, and history bounding are implemented at `crates/executive/src/service/context_assembler.rs:13-227`. The live adapter reads the cached prefix, unified `MemoryService`, typed fact use cases, core memory, skill catalog/router, Dasein view, and Agora snapshot at `crates/executive/src/service/context_assembler.rs:45-174`; bootstrap supplies those handles without a `CoreSystems` lookup at `crates/executive/src/impl/daemon/handler/init.rs`. `TurnPipeline` delegates context assembly once and now replays model history from the shared canonical `SessionService` at `crates/executive/src/service/turn_pipeline.rs:422-445`; canonical resume/fork/interrupt/replay RPCs reuse that same service at `crates/executive/src/impl/daemon/handler/mod.rs:310-318`. The duplicate daemon/turn injection module was deleted. Focused context, canonical lifecycle, turn-order, and daemon/exec equivalence tests pass. The task remains open because the remaining legacy clear/list/resume/compact/new/load-recent RPC and routing paths have not yet migrated.

### Task 6: Extract synchronous turn adapters and post-turn projections

**Files:**
- Create: `crates/executive/src/service/turn_runtime_ports.rs`
- Create: `crates/executive/src/service/post_turn_projection.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/service/daemon_turn/post_phases.rs`
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs`
- Test: `crates/executive/tests/turn_use_case_ports.rs`

- [ ] Define narrow ports for hook execution, storm-breaker state, model selection, Self policy, governed capability execution, and approval; keep capability/approval calls synchronous in the turn.
- [ ] Define `PostTurnProjection::project` with a bounded immutable turn outcome; move non-blocking memory, Agora trace, reflection, and evolution updates behind the projection port.
- [ ] Make projection failure observable but non-destructive to an already-settled turn; keep approval/capability failure turn-blocking.
- [ ] Test ordering `context -> model -> governed capability -> terminal settlement -> projection`, projection outage behavior, and no direct domain lock from `TurnPipeline`.

### Task 7: Build private HandlerPorts and remove CoreSystems from RequestHandler

**Files:**
- Create: `crates/executive/src/impl/daemon/handler/ports.rs`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_health.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_reflection.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_google.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_turn.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_workflow.rs`
- Test: `crates/executive/tests/request_use_case_boundaries.rs`

- [ ] Define private `HandlerPorts` fields for facts, goals, approvals, admin, sessions, health, reflection, Google, workflow, and turn control. Every field is `Arc<dyn ...UseCases>` or an already-narrow service; no concrete group/store appears.
- [ ] In bootstrap, construct concrete adapters from existing handles once, then build `HandlerPorts`; do not let individual handlers construct adapters.
- [ ] Replace `RequestHandler.subsystems` with `ports`; keep connection-local fields (`sessions`, notify channel, connection count, lifecycle task handles) only where they are protocol state rather than domain state.
- [ ] Migrate remaining RPC files and dead fallback helpers, deleting unused direct-bus/domain-lock paths rather than retaining a second route.
- [ ] Run the request-boundary test; expected PASS with zero forbidden hits.

### Task 8: Validation and X02 handoff

**Files:**
- Modify: `config/architecture-allowlist.txt`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [ ] Prove `rg -n 'subsystems|CoreSystems|MemoryGroup|CorpusGroup|SecurityGroup|FactStore|ObjectiveStore|ApprovalRepository|\.lock\(\)' crates/executive/src/impl/daemon/handler/rpc` returns no production hits.
- [ ] Prove `RequestHandler` has no `CoreSystems` field and every RPC family receives only a use-case port.
- [ ] Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, all focused X01 tests, `cargo test --workspace`, and `scripts/architecture-check.sh`.
- [ ] Mark X01 done in the coverage matrix only after JSON-RPC compatibility, daemon/exec equivalence, cancellation, approval, and projection failure tests pass.
- [ ] Record the only remaining god-container/bootstrap references as X02 input; do not claim X02 complete until `CoreSystems` is private/deleted and `init.rs` is split.

**Deletion gate:** X01 is incomplete while any JSON-RPC handler reaches `CoreSystems`, a concrete domain store/registry, or a domain mutex; while `TurnPipeline` assembles context or performs post-turn store writes directly; or while an alternative approval/capability route exists outside the governed services.
