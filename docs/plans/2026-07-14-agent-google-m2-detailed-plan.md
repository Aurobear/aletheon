# Aletheon M2 Persistent Goal Runtime Detailed Implementation Plan

> **For agentic workers:** Implement one numbered task at a time. Stop after its scoped tests and commit gate.

**Goal:** Evolve the existing ObjectiveStore into a restart-safe single-active-Goal runtime with immutable intent, explicit Goal state, bounded ticks, budgets, and Telegram/RPC lifecycle commands.

**Architecture:** Preserve `objectives.db`, existing ObjectiveStore CRUD, existing RPC methods, and startup `seed_goal()` behavior. Add compatible columns and event/ledger tables, then place a `GoalCoordinator` above the repository while keeping generic kernel `ProcessState` unchanged.

**Tech Stack:** Rust, Tokio, rusqlite, serde/serde_json, existing `ProcessTable`, `OperationTable`, `Clock`, ObjectiveStore, daemon RPC, and the M1 ChannelRouter.

---

## 1. Requirement and code anchors

Requirements:

- Persistent Goal fields and state: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:6-44`.
- Immutable original intent: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:46-81`.
- Bounded `tick()`: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:83-125`.
- Status/pause/resume/cancel: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:343-369`.
- One active Goal surviving restart: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:371-385` and `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:72-81`.

Current implementation to preserve:

- Objective ABI: `crates/fabric/src/types/objective.rs:9-79`.
- ObjectiveStore schema/opening: `crates/executive/src/impl/goal/mod.rs:12-69`.
- Existing CRUD/recovery: `crates/executive/src/impl/goal/store.rs:15-111`.
- Existing goal RPC API: `crates/executive/src/impl/daemon/handler/rpc/rpc_goal.rs:9-141`.
- Startup recovery and `seed_goal()`: `crates/executive/src/impl/daemon/handler/init.rs:249-278`, `:375-377`.
- Existing in-memory Cognit GoalTracker is turn context, not durable authority: `crates/cognit/src/harness/linear/goal_tracker.rs:45-77`.
- Kernel process lifecycle remains generic: `crates/fabric/src/types/process.rs:60-90`.

M2 does **not** implement DeepSeek retry, Pi, verification, durable approval, or multi-Goal scheduling. Draft confirmation is represented in state and may be exercised through trusted command tests; M5 later supplies restart-safe approval requests.

## 2. Task 1 — Add Goal domain types compatibly

**Files:**

- Create: `crates/fabric/src/types/goal.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

### Step 1.1: Write state tests first

- [ ] Add tests covering serde and this transition table:

```rust
#[test]
fn goal_state_transitions_are_explicit() {
    use GoalState::*;
    assert!(Draft.can_transition_to(Ready));
    assert!(Ready.can_transition_to(Running));
    assert!(Running.can_transition_to(Blocked));
    assert!(Running.can_transition_to(AwaitingHuman));
    assert!(Running.can_transition_to(Suspended));
    assert!(Suspended.can_transition_to(Ready));
    assert!(Running.can_transition_to(Completed));
    assert!(Running.can_transition_to(Failed));
    assert!(Draft.can_transition_to(Cancelled));
    assert!(!Completed.can_transition_to(Running));
    assert!(!Cancelled.can_transition_to(Ready));
}
```

### Step 1.2: Define the M2 types

- [ ] Implement:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Draft,
    Ready,
    Running,
    Blocked,
    AwaitingHuman,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalWaitReason {
    HumanInput { prompt: String },
    ExternalEvent { key: String },
    Backoff { until_ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_cost_usd: Option<f64>,
    pub max_attempts: u32,
    pub deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalBudgetUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalSpec {
    pub original_intent: String,
    pub desired_state: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub budget: GoalBudget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalSnapshot {
    pub id: GoalId,
    pub owner: PrincipalId,
    pub state: GoalState,
    pub spec: GoalSpec,
    pub usage: GoalBudgetUsage,
    pub wait_reason: Option<GoalWaitReason>,
    pub process_id: Option<ProcessId>,
    pub version: u64,
    pub created_at: String,
    pub updated_at: String,
}
```

- [ ] Implement `GoalState::as_str`, `from_str`, `is_terminal`, and `can_transition_to` without changing `ProcessState`.
- [ ] Re-export all types from `fabric::types` and `fabric`.

### Step 1.3: Validate and commit

- [ ] Run `cargo test -p fabric -- types::goal` and `cargo check -p fabric`; expect exit 0.
- [ ] Commit with subject `feat(fabric): define persistent goal contracts` and a body noting Goal state is separate from kernel process state.

## 3. Task 2 — Introduce ObjectiveStore schema migrations

**Files:**

- Create: `crates/executive/src/impl/goal/migrations.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

### Step 2.1: Protect the legacy database

- [ ] Write a test that manually creates the exact legacy `objectives` schema from `crates/executive/src/impl/goal/mod.rs:31-46`, inserts a row, reopens with the new ObjectiveStore, and proves the row remains readable.
- [ ] Write a second test that opens the same migrated database twice and observes the same schema version.

### Step 2.2: Implement versioned migration without renaming tables

- [ ] Keep `objectives` and its legacy columns. Add these columns only when absent:

```sql
owner_id       TEXT NOT NULL DEFAULT 'local-owner';
goal_state     TEXT NOT NULL DEFAULT 'ready';
spec_json      TEXT NOT NULL DEFAULT '{}';
wait_json      TEXT;
process_id     TEXT;
version        INTEGER NOT NULL DEFAULT 0;
deadline_ms    INTEGER;
```

- [ ] Add:

```sql
CREATE TABLE IF NOT EXISTS goal_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(objective_id, version)
);

CREATE TABLE IF NOT EXISTS goal_budget_ledger (
    ledger_id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    reservation_id TEXT NOT NULL UNIQUE,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK(status IN ('reserved','settled','revoked')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    settled_at TEXT
);
```

- [ ] Record migration version in `PRAGMA user_version`; run each migration in a transaction.
- [ ] Move current `CREATE TABLE IF NOT EXISTS objectives` into migration 1 and extended schema into migration 2.

### Step 2.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::goal`; expect legacy, fresh, and repeated-open tests to pass.
- [ ] Commit with subject `feat(executive): migrate objective storage for goals` and document compatibility with existing `objectives.db`.

## 4. Task 3 — Map Objective and GoalSnapshot without breaking RPC

**Files:**

- Modify: `crates/executive/src/impl/goal/mod.rs`
- Modify: `crates/executive/src/impl/goal/store.rs`

### Step 3.1: Add compatibility tests

- [ ] Prove existing methods retain behavior and signatures:

```text
create(description, parent, session, scope) -> i64
get(id) -> Objective
set_status(id, legacy_status)
list(filter, limit)
active()
sub_goals(parent)
resume()
```

- [ ] Add `create_goal`, `get_goal`, and `list_goals` tests proving `original_intent`, owner, state, budget, and version round-trip.

### Step 3.2: Add the new API beside the legacy API

- [ ] Implement:

```rust
pub fn create_goal(
    &mut self,
    owner: &PrincipalId,
    session_id: &str,
    scope: &str,
    spec: &GoalSpec,
) -> anyhow::Result<GoalSnapshot>;

pub fn get_goal(&self, id: GoalId) -> anyhow::Result<Option<GoalSnapshot>>;
pub fn list_goals(&self, states: &[GoalState], limit: usize) -> anyhow::Result<Vec<GoalSnapshot>>;
pub fn recoverable_goals(&self) -> anyhow::Result<Vec<GoalSnapshot>>;
```

- [ ] `create_goal()` inserts an `objectives` row plus version-0 `goal_events` entry in one transaction.
- [ ] Legacy `create()` continues producing a Ready GoalSpec whose immutable original intent equals `description`.
- [ ] Never overwrite `spec.original_intent` in an update path.

### Step 3.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::goal` and existing RPC goal tests; expect exit 0.
- [ ] Commit with subject `feat(executive): expose compatible goal repository API`.

## 5. Task 4 — Add optimistic atomic transitions

**Files:**

- Create: `crates/executive/src/impl/goal/transition.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`
- Modify: `crates/executive/src/impl/goal/store.rs`

### Step 4.1: Write transition tests

- [ ] Test legal transition success, illegal transition rejection, stale-version conflict, immutable intent, terminal-state rejection, event insertion, and rollback when event insertion fails.

### Step 4.2: Implement typed transition errors

- [ ] Define:

```rust
pub enum GoalTransitionError {
    NotFound(GoalId),
    Illegal { from: GoalState, to: GoalState },
    VersionConflict { expected: u64, actual: u64 },
    Storage(String),
}
```

- [ ] Implement:

```rust
pub fn transition_goal(
    &mut self,
    id: GoalId,
    expected_version: u64,
    next: GoalState,
    wait_reason: Option<&GoalWaitReason>,
    event_payload: &serde_json::Value,
) -> Result<GoalSnapshot, GoalTransitionError>;
```

Within one transaction, update only when `version = expected_version`, increment version, and insert the same version in `goal_events`.

### Step 4.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::goal::transition`; expect PASS.
- [ ] Commit with subject `feat(executive): make goal transitions atomic`.

## 6. Task 5 — Add durable Goal budget reservations

**Files:**

- Create: `crates/executive/src/impl/goal/budget.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

### Step 5.1: Write ledger tests

- [ ] Test reservation within limits, input/output token exhaustion, cost exhaustion, attempt exhaustion, deadline expiry using a fake clock value, duplicate reservation ID, settlement, revoke, and restart reconstruction.

### Step 5.2: Implement repository-scoped ledger

- [ ] Define `GoalBudgetRequest`, `GoalBudgetReservation`, and `GoalBudgetError`.
- [ ] Implement:

```rust
pub fn reserve_goal_budget(
    &mut self,
    id: GoalId,
    request: GoalBudgetRequest,
    now_ms: i64,
) -> Result<GoalBudgetReservation, GoalBudgetError>;

pub fn settle_goal_budget(
    &mut self,
    reservation_id: &str,
    actual: GoalBudgetUsage,
) -> Result<(), GoalBudgetError>;

pub fn revoke_goal_budget(&mut self, reservation_id: &str) -> Result<(), GoalBudgetError>;
```

- [ ] Sum settled plus active reservations before admitting work. This ledger supplements, and does not replace, capability `AdmissionController`.

### Step 5.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::goal::budget`; expect PASS.
- [ ] Commit with subject `feat(executive): persist goal budget accounting`.

## 7. Task 6 — Implement a bounded GoalCoordinator

**Files:**

- Create: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`
- Create: `crates/executive/tests/goal_lifecycle.rs`

### Step 6.1: Define one-tick outcome

- [ ] Define:

```rust
pub enum GoalTickOutcome {
    Noop { state: GoalState },
    Transitioned { from: GoalState, to: GoalState },
    TurnRequested { goal_id: GoalId, input: String },
    BudgetBlocked { reason: String },
}
```

### Step 6.2: Test bounded behavior

- [ ] Using a fake repository/turn executor, prove one `tick()` causes at most one transition or one turn request. Also prove Draft/Suspended/terminal states do no work, deadline failure is deterministic, and a second tick must be explicitly scheduled.

### Step 6.3: Implement coordinator policy

- [ ] Implement `tick(goal_id)`:

```text
load current snapshot
terminal/draft/suspended/awaiting-human -> Noop
ready -> transition to Running only
running -> reserve one attempt budget and return one TurnRequested
blocked -> Noop
```

M2 does not loop and does not call a worker itself. M3 consumes `TurnRequested` with per-runtime attempts.

### Step 6.4: Validate and commit

- [ ] Run `cargo test -p executive --test goal_lifecycle`; expect PASS.
- [ ] Commit with subject `feat(executive): coordinate bounded goal ticks`.

## 8. Task 7 — Link live Goals to kernel processes

**Files:**

- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/goal/store.rs`
- Modify: `crates/executive/tests/goal_lifecycle.rs`

### Step 7.1: Add linkage tests

- [ ] Test Ready→Running creates one `ProcessTable` record, persists its ProcessId, pause maps the live process to `Waiting`, resume maps it back to `Running`, cancel cancels the active operation before marking Goal Cancelled, and restart clears a stale process ID rather than assuming the old process exists.

### Step 7.2: Implement linkage API

- [ ] Add `set_process_link(goal_id, expected_version, Option<ProcessId>)` as an atomic versioned event.
- [ ] Use `AgentProfileId("goal")`, `NamespaceId(format!("goal:{}", id.0))`, and `OperationKind::Turn` for the live process.
- [ ] Keep Goal state authoritative when kernel process state and SQLite disagree after restart.

### Step 7.3: Validate and commit

- [ ] Run `cargo test -p executive --test goal_lifecycle`; expect PASS.
- [ ] Commit with subject `feat(executive): link goals to live kernel processes`.

## 9. Task 8 — Recover Goals at daemon startup

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/src/impl/goal/store.rs`
- Create: `crates/executive/tests/goal_restart_recovery.rs`

### Step 8.1: Add recovery cases

- [ ] Cover Draft, Ready, Running-with-stale-process, Suspended, AwaitingHuman, Completed, and legacy in-progress Objective rows.

Expected policy:

```text
Draft          -> remains Draft
Ready          -> remains Ready
Running        -> clear process link; become Ready; append recovered event
Suspended      -> remains Suspended
AwaitingHuman  -> remains AwaitingHuman
terminal       -> not scheduled
legacy active  -> map to Ready and continue current seed_goal compatibility
```

### Step 8.2: Replace single-row resume assumptions carefully

- [ ] Keep `ObjectiveStore::resume()` for backward compatibility.
- [ ] Add `recover_goals()` for new stateful recovery and call it at initialization.
- [ ] Continue seeding the current Cognit GoalTracker from the selected active Goal until M3 introduces GoalFrame construction.

### Step 8.3: Validate and commit

- [ ] Run `cargo test -p executive --test goal_restart_recovery` plus `cargo test -p executive -- impl::goal`; expect PASS.
- [ ] Commit with subject `fix(executive): recover persistent goal state on restart`.

## 10. Task 9 — Extend RPC without breaking legacy methods

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/rpc.rs`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_goal.rs`
- Test: existing RPC tests and a new `crates/executive/tests/goal_rpc.rs`

### Step 9.1: Preserve and extend API

- [ ] Preserve `goal.set`, `goal.show`, `goal.status`, and `goal.resume` response compatibility.
- [ ] Add `goal.create`, `goal.list`, `goal.pause`, `goal.run`, and `goal.cancel`.
- [ ] Reject empty intent, invalid IDs, illegal transitions, stale versions, and a second active top-level Goal.

### Step 9.2: Return GoalSnapshot for new methods

- [ ] New methods return structured state/version/budget. Legacy methods continue returning Objective/ObjectiveSummary shapes.
- [ ] Route all mutations through `transition_goal()` or coordinator methods; do not call raw SQL or legacy `set_status()` for new APIs.

### Step 9.3: Validate and commit

- [ ] Run `cargo test -p executive --test goal_rpc`; expect PASS.
- [ ] Commit with subject `feat(executive): expose persistent goal lifecycle RPC`.

## 11. Task 10 — Enable Telegram Goal commands

**Files:**

- Modify: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/impl/channel/daemon_adapter.rs`
- Create: `crates/executive/tests/telegram_goal_commands.rs`

### Step 10.1: Add a Goal command boundary

- [ ] Extend the M1 router with an injected `ChannelGoalExecutor` trait:

```rust
#[async_trait::async_trait]
pub trait ChannelGoalExecutor: Send + Sync {
    async fn create_draft(&self, owner: &str, intent: &str) -> anyhow::Result<GoalSnapshot>;
    async fn list(&self, owner: &str) -> anyhow::Result<Vec<GoalSnapshot>>;
    async fn show(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn pause(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn resume(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn cancel(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
}
```

### Step 10.2: Test commands

- [ ] Test `/goal`, `/goals`, `/status`, `/pause`, `/resume`, and `/cancel`, including wrong owner, missing argument, malformed ID, replayed Telegram update, and second-active-Goal rejection.
- [ ] `/goal` creates Draft with immutable original intent. In M2, `/resume <draft-id>` acts as trusted-owner confirmation and transitions Draft→Ready; M5 later replaces this shortcut with durable approvals.

### Step 10.3: Validate and commit

- [ ] Run `cargo test -p executive --test telegram_goal_commands`; expect PASS.
- [ ] Commit with subject `feat(executive): add Telegram goal lifecycle commands`.

## 12. Task 11 — M2 end-to-end restart validation

**Files:**

- Create: `crates/executive/tests/persistent_goal_vertical_slice.rs`

### Step 11.1: Test the full M2 slice

- [ ] Use a temporary `objectives.db`, fake channel, fake clock, and real kernel test ports:

```text
/goal ship feature -> Draft
/resume id          -> Ready
tick                 -> Running with ProcessId
pause                -> Suspended / process Waiting
daemon reconstruction
recover              -> Suspended with no stale execution
resume               -> Ready
tick                 -> Running with new ProcessId
cancel               -> Cancelled and operation cancelled
restart              -> remains Cancelled and is not scheduled
```

- [ ] Verify original intent never changes and every state mutation has monotonically increasing version/event rows.

### Step 11.2: Run deterministic release commands

- [ ] Run:

```bash
cargo fmt --all -- --check
cargo test -p fabric -- types::goal
cargo test -p executive -- impl::goal
cargo test -p executive --test goal_lifecycle
cargo test -p executive --test goal_restart_recovery
cargo test -p executive --test goal_rpc
cargo test -p executive --test telegram_goal_commands
cargo test -p executive --test persistent_goal_vertical_slice
cargo test --workspace
cargo build --workspace
```

Expected: every command exits 0.

### Step 11.3: Inspect invariants

- [ ] Confirm no Goal-specific variant was added to `ProcessState`.
- [ ] Confirm no `goals` table or parallel Goal database was created.
- [ ] Confirm legacy `goal.*` RPC and ObjectiveStore tests still pass.
- [ ] Confirm no unbounded coordinator loop exists.
- [ ] Confirm all new mutations use optimistic versions and event transactions.

## 13. DeepSeek execution batches

Recommended batches:

1. Task 1 only — shared ABI.
2. Tasks 2–3 — migration and compatible mapping.
3. Tasks 4–5 — transitions and budget ledger.
4. Tasks 6–7 — bounded coordinator and kernel linkage.
5. Tasks 8–9 — recovery and RPC.
6. Tasks 10–11 — Telegram and end-to-end validation.

For every batch:

```text
Write only the listed files.
Do not modify ProcessState.
Do not create a new GoalRepository or goals table.
Do not delete or rename ObjectiveStore APIs.
Write the named failing test first.
Run the scoped test and report its exact exit code.
Inspect the diff before committing.
Stop after the batch; do not begin M3.
```
