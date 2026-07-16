# G03 Transactional Agent Control Service Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one durable Executive `AgentControlService` the authority for Agent spawn, wait, send, cancel, inspect and list operations.

**Architecture:** Preserve the Fabric `AgentControlPort` contract and opaque `KernelRuntime`. A SQLite repository commits the run identity and parent edge before one runtime task starts; compensating cleanup releases Kernel resources if persistence or launch fails. `SubAgentSpawner` becomes a compatibility adapter and is deleted after Corpus tools and goal workers use the service.

**Tech Stack:** Rust, Tokio, rusqlite, async-trait, KernelRuntime, Fabric Agent contracts

**Prerequisites:** G02, K02, X02.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:453-506`; facade acceptance at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1100-1111`.

---

## Current-code anchors

- The complete control contract already exists at `crates/fabric/src/types/agent_control.rs:8-304`.
- Runtime selection and live state are still held in `SubAgentSpawner` at `crates/executive/src/core/sub_agent.rs:153-176`.
- Spawn creates Kernel resources directly in the compatibility spawner at `crates/executive/src/core/sub_agent.rs:248-330` and `:393-449`.
- Wait is a compatibility operation on live in-memory entries at `crates/executive/src/core/sub_agent.rs:712`.
- `KernelRuntime` exposes opaque process spawn, operation cancellation and waits at `crates/kernel/src/runtime.rs:308`, `:644`, `:750-762`.

## Invariants and non-goals

- Resolve the runtime and reserve admission before creating Process/Operation/Space/mailbox resources.
- Persist one `AgentId -> ProcessId -> OperationId` identity; retries never create a second run.
- Provider retries belong to the runtime; `SupervisorTree` handles only process crash policy.
- Terminal metadata remains queryable after live task eviction.
- Do not add a second Kernel table, mailbox registry, Agent contract, or polling wait loop.

## Key contracts

### Task 1: Lock the authoritative service and repository contract

**Files:**
- Create: `crates/executive/src/service/agent_control/mod.rs`
- Create: `crates/executive/src/service/agent_control/repository.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Create: `crates/executive/tests/agent_control_service.rs`

- [ ] Add a failing static test proving production `AgentControlPort` is implemented only by `AgentControlService` and Corpus does not import `SubAgentSpawner`.
- [ ] Define the repository boundary:

```rust
#[async_trait::async_trait]
pub trait AgentRunRepository: Send + Sync {
    async fn create(&self, run: &AgentRunRecord) -> Result<(), AgentControlError>;
    async fn transition(&self, agent: AgentId, expected: AgentRunStatus, next: AgentRunStatus, result: Option<AgentResult>, error: Option<String>, now_ms: i64) -> Result<AgentRunRecord, AgentControlError>;
    async fn get(&self, agent: AgentId) -> Result<Option<AgentRunRecord>, AgentControlError>;
    async fn list_root(&self, root: AgentId, status: Option<AgentRunStatus>, limit: usize) -> Result<Vec<AgentRunRecord>, AgentControlError>;
}
```

- [ ] Define `AgentRuntimeLauncher::launch(input: AgentRuntimeInput, events: Arc<dyn AgentEventSink>)` and inject it, `Arc<KernelRuntime>`, `Arc<dyn Clock>`, repository and admission port into `AgentControlService`.
- [ ] Run `cargo test -p executive --test agent_control_service`; expect the static test to fail until Tasks 2-4 land.
- [ ] Commit with subject `test(agent): lock authoritative control service` and a body describing the duplicate spawner path.

### Task 2: Persist runs and parent edges transactionally

**Files:**
- Create: `crates/executive/src/service/agent_control/sqlite_repository.rs`
- Create: `crates/executive/src/service/agent_control/migrations/001_agent_runs.sql`
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Test: `crates/executive/tests/agent_control_repository.rs`

- [ ] Add reopen, duplicate-ID, parent-edge, compare-and-swap transition, root-scoped list and bounded-result tests.
- [ ] Create `agent_runs` with the columns from source lines 480-499 plus `request_hash`, `version`, and `retain_until_ms`; create `(root_agent_id,status,created_at_ms)` and `(parent_agent_id,created_at_ms)` indexes.
- [ ] Use `BEGIN IMMEDIATE`; insert the run and parent edge in one transaction and reject duplicate `agent_id` or mismatched request hash as `Conflict`.
- [ ] Encode `AgentRunStatus` as explicit snake-case strings and reject unknown stored values as `Persistence`, never as `Queued`.
- [ ] Run `cargo test -p executive --test agent_control_repository`; expect all restart and CAS cases to pass.
- [ ] Commit with subject `feat(agent): persist durable run metadata`.

### Task 3: Implement compensating spawn and one owned runtime task

**Files:**
- Create: `crates/executive/src/service/agent_control/admission.rs`
- Create: `crates/executive/src/service/agent_control/execution.rs`
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Test: `crates/executive/tests/agent_control_spawn.rs`

- [ ] Test failure at runtime resolution, admission, Kernel spawn, repository create and launcher start; assert each earlier resource is released exactly once.
- [ ] Validate `AgentSpawnRequest`, resolve `RuntimeId`, reserve admission, spawn the Kernel child under `parent_process_id`, create its operation and mailbox, then persist `Queued`.
- [ ] Transfer the admission lease and `OperationScope` into one task; transition `Queued -> Running -> terminal`, persist bounded `AgentResult`, and call the Kernel terminal transaction once.
- [ ] On failure before task ownership transfer, cancel the operation, terminate the process and release admission in reverse construction order.
- [ ] Run `cargo test -p executive --test agent_control_spawn`; expect all failpoint and exactly-once assertions to pass.
- [ ] Commit with subject `feat(agent): centralize transactional spawn`.

### Task 4: Implement wait, send, cancel, inspect and list

**Files:**
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Create: `crates/executive/src/service/agent_control/live_runs.rs`
- Test: `crates/executive/tests/agent_control_operations.rs`

- [ ] Store a `watch::Sender<AgentSnapshot>` per live run and implement wait with `Timer::timeout`, not database polling.
- [ ] Authorize every operation by comparing `caller_root_agent_id` to the persisted root.
- [ ] Route send through the Kernel mailbox; reject terminal recipients as `Terminal`; persist the message reference before acknowledging delivery.
- [ ] Make cancel idempotent, cancel the operation once, and return the persisted terminal snapshot.
- [ ] Serve inspect/list from the repository after live eviction and enforce `MAX_LIST_ITEMS`.
- [ ] Run `cargo test -p executive --test agent_control_operations`; expect root isolation, timeout, idempotence and post-eviction cases to pass.
- [ ] Commit with subject `feat(agent): expose durable control operations`.

### Task 5: Wire bootstrap and retire direct production spawning

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Construct one `Arc<dyn AgentControlPort>` in bootstrap and pass it to runtime/tool adapters.
- [ ] Keep `SubAgentSpawner` only behind a compatibility adapter for goal workers until G04/G05, with a deletion gate rejecting new imports.
- [ ] Add shutdown draining so service-owned tasks settle repository state before daemon exit.
- [ ] Run `cargo test -p executive --test agent_control_service --test agent_control_spawn --test agent_control_operations` and `scripts/architecture-check.sh`; expect PASS and no new finding.
- [ ] Commit with subject `refactor(agent): route production through control service`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] One service implements every `AgentControlPort` method.
- [ ] Spawn is compensating and starts exactly one operation-owned runtime task.
- [ ] Wait uses subscription; terminal rows survive live eviction and restart.
- [ ] Cross-root access, duplicate spawn and terminal send fail closed.
- [ ] Focused tests, workspace Clippy/tests and architecture fitness pass.
