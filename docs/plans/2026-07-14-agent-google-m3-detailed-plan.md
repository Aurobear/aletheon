# Aletheon M3 Runtime, Attempts, Retry, and Escalation Detailed Plan

> **For agentic workers:** Implement one numbered task at a time and stop after its commit gate.

**Goal:** Add per-agent runtime selection and durable Goal attempts so DeepSeek can execute bounded work, retry with evidence, and escalate to a distinct reviewer runtime without abusing SupervisorTree.

**Architecture:** `RuntimeRegistry` selects an `Arc<dyn SubAgentRuntime>` per spawn while preserving the current default-runtime API. `AttemptCoordinator` persists one attempt per runtime call, classifies its structured result, settles Goal budget, and explicitly schedules retry/escalation.

**Tech Stack:** Rust, Tokio, existing `SubAgentSpawner`, `LlmProvider`, ObjectiveStore Goal extensions from M2, ProcessTable/OperationTable, serde, SQLite.

---

## 1. Anchors and boundaries

- Attempts and evidence: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:166-181`.
- Failure classes: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:183-217`.
- Three-attempt bounded escalation: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:219-237`.
- Native/DeepSeek/Pi/reviewer roles: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:270-314`.
- Current runtime trait: `crates/executive/src/core/sub_agent.rs:42-53`.
- Current global runtime limitation: `crates/executive/src/core/sub_agent.rs:119-130`, `:190-197`.
- Current daemon runtime already performs provider/tool steps: `crates/executive/src/impl/daemon/handler/init.rs:69-130`.
- SupervisorTree restarts kernel processes only: `crates/kernel/src/supervision/tree.rs:8-51`.

M3 does not implement Pi worktrees or verification commands. It defines the contracts M4 will consume.

## 2. Task 1 — Define runtime and attempt contracts

**Files:**

- Create: `crates/fabric/src/types/attempt.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Write serde tests, then define:

```rust
pub struct RuntimeId(pub String);
pub struct AttemptId(pub uuid::Uuid);

pub enum CognitiveRole {
    Worker,
    Reviewer,
    Debugger,
    Verifier,
}

pub enum FailureClass {
    Compilation,
    TestFailure,
    PermissionDenied,
    Timeout,
    MissingDependency,
    InvalidAssumption,
    ArchitectureViolation,
    ToolFailure,
    ContextInsufficient,
    ProviderTransient,
    ProviderPermanent,
    Cancelled,
    RepeatedFailure,
}

pub struct AttemptUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub elapsed_ms: u64,
}

pub enum AttemptStatus { Running, Succeeded, Failed, Cancelled }

pub struct AttemptEvidence {
    pub kind: String,
    pub summary: String,
    pub content: String,
}

pub struct RuntimeResult {
    pub output: String,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
}

pub struct RuntimeFailure {
    pub class: FailureClass,
    pub message: String,
    pub retryable: bool,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
}
```

- [ ] Keep provider error details redacted and bounded before persistence.
- [ ] Run `cargo test -p fabric -- types::attempt`; expect PASS.
- [ ] Commit `feat(fabric): define runtime attempt contracts` with a compatibility rationale.

## 3. Task 2 — Evolve SubAgentRuntime without breaking callers

**Files:**

- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: existing subagent test doubles found with `rg -n "impl SubAgentRuntime" crates/executive`

- [ ] First add a compile test proving the legacy `run(&str, CancellationToken) -> Result<String,String>` path still works.
- [ ] Add a richer default method rather than immediately breaking all implementations:

```rust
#[async_trait]
pub trait SubAgentRuntime: Send + Sync {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>;

    async fn run_attempt(
        &self,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.run(task, cancel).await
            .map(|output| RuntimeResult { output, usage: Default::default(), evidence: vec![] })
            .map_err(|message| RuntimeFailure {
                class: FailureClass::ToolFailure,
                message,
                retryable: false,
                usage: Default::default(),
                evidence: vec![],
            })
    }
}
```

- [ ] New runtimes override `run_attempt`; old test/dev implementations remain valid.
- [ ] Run `cargo test -p executive --test supervision`; expect PASS.
- [ ] Commit `refactor(executive): add structured subagent attempt results`.

## 4. Task 3 — Add RuntimeRegistry and per-spawn selection

**Files:**

- Create: `crates/executive/src/core/runtime_registry.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Test: `crates/executive/tests/runtime_registry.rs`

- [ ] Test duplicate ID rejection, missing ID, two concurrent distinct runtimes, default-runtime compatibility, and runtime ID retained in the subagent entry.
- [ ] Implement:

```rust
pub struct RuntimeRegistry {
    runtimes: HashMap<RuntimeId, Arc<dyn SubAgentRuntime>>,
}

impl RuntimeRegistry {
    pub fn register(&mut self, id: RuntimeId, runtime: Arc<dyn SubAgentRuntime>) -> anyhow::Result<()>;
    pub fn resolve(&self, id: &RuntimeId) -> anyhow::Result<Arc<dyn SubAgentRuntime>>;
}
```

- [ ] Add `runtime_registry` to `SubAgentSpawner` and preserve `with_runtime()` by registering/setting a reserved `RuntimeId("default")`.
- [ ] Add `spawn_with_runtime(task, parent_turn_id, runtime_id, restart_policy)`; resolve the runtime before creating process/operation records.
- [ ] Save `runtime_id` on `SubAgentEntry` so unexpected process restart uses the same runtime only.
- [ ] Do not teach SupervisorTree about providers.
- [ ] Run `cargo test -p executive --test runtime_registry --test supervision`; expect PASS.
- [ ] Commit `feat(executive): select subagent runtime per spawn`.

## 5. Task 4 — Move daemon runtime into a reusable provider runtime

**Files:**

- Create: `crates/executive/src/impl/runtime/mod.rs`
- Create: `crates/executive/src/impl/runtime/provider_worker.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`

- [ ] Extract the current `DaemonSubAgentRuntime` loop from `init.rs:69-159` without changing tool behavior.
- [ ] Parameterize `ProviderWorkerRuntime` by `RuntimeId`, role, `Arc<dyn LlmProvider>`, tool registry, max steps, and allowed tool names.
- [ ] Override `run_attempt()` to return actual `LlmResponse.usage` from `crates/fabric/src/types/llm_types.rs:93-115` and elapsed time from the injected `Clock`.
- [ ] Bound persisted output/evidence to configured byte limits and classify cancellation separately.
- [ ] Write fake-provider tests for end-turn, tool-use loop, max-step exhaustion, cancellation, usage aggregation, and tool allow-list.
- [ ] Keep a `run()` adapter returning only output/error for compatibility.
- [ ] Run `cargo test -p executive -- impl::runtime::provider_worker`; expect PASS.
- [ ] Commit `refactor(executive): extract provider worker runtime`.

## 6. Task 5 — Register DeepSeek and reviewer runtimes from config

**Files:**

- Modify: `crates/cognit/src/config/mod.rs`
- Modify: `crates/executive/src/core/runtime_core.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `docs/design/executive/daemon.md`

- [ ] Add role routing config containing runtime ID and model/provider alias; defaults must not silently claim a DeepSeek provider exists.
- [ ] At bootstrap, resolve configured worker and reviewer providers through the existing ProviderRegistry.
- [ ] Register `deepseek-worker` and `escalation-reviewer` only when configured; missing required runtime produces a startup/config error when Goal execution is enabled.
- [ ] Test disabled configuration, missing alias, same provider under different runtime IDs, and successful distinct-provider registration.
- [ ] Never branch on provider brand inside `ProviderWorkerRuntime`; “DeepSeek” is configuration, not a hard-coded protocol.
- [ ] Run config/bootstrap scoped tests and `cargo check -p executive`; expect PASS.
- [ ] Commit `feat(executive): configure worker and reviewer runtimes`.

## 7. Task 6 — Persist M3 attempt records in ObjectiveStore

**Files:**

- Modify: `crates/executive/src/impl/goal/migrations.rs`
- Create: `crates/executive/src/impl/goal/attempt.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

- [ ] Add migration:

```sql
CREATE TABLE goal_attempts (
    attempt_id TEXT PRIMARY KEY,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    runtime_id TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','cancelled')),
    input_json TEXT NOT NULL,
    output_json TEXT,
    failure_json TEXT,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    usage_json TEXT NOT NULL DEFAULT '{}',
    started_at TEXT NOT NULL,
    ended_at TEXT,
    UNIQUE(objective_id, sequence)
);
```

- [ ] Write tests for create-running, finish-success, finish-failure, cancel, duplicate sequence, immutable input/runtime ID, list newest attempts, and reopen recovery of a stale running attempt.
- [ ] Implement `begin_attempt`, `finish_attempt`, `cancel_attempt`, `attempts_for_goal`, and `recover_stale_attempts` with transactions and Goal event entries.
- [ ] Mark a stale `running` attempt as failed/cancelled on daemon restart; never re-run it invisibly.
- [ ] Run `cargo test -p executive -- impl::goal::attempt`; expect PASS.
- [ ] Commit `feat(executive): persist goal attempts and evidence`.

## 8. Task 7 — Define retry and escalation policy

**Files:**

- Create: `crates/executive/src/impl/goal/retry.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

- [ ] Define:

```rust
pub struct RetryPolicy {
    pub max_worker_attempts: u32,
    pub max_reviewer_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

pub enum RetryDecision {
    RetrySame { after_ms: u64, evidence: Vec<AttemptEvidence> },
    Escalate { runtime_id: RuntimeId, evidence: Vec<AttemptEvidence> },
    AwaitHuman { reason: String },
    Fail { reason: String },
    Cancel,
}
```

- [ ] Table-test every `FailureClass` and attempt count. Permission/auth/policy failures must not retry blindly; cancellation never retries; transient errors back off; repeated compile/test failures carry evidence; third repeated worker failure escalates; exhausted reviewer attempts await human/fail.
- [ ] Backoff uses persisted wall-clock deadline/GoalWaitReason and injected clock, not `std::thread::sleep`.
- [ ] Run `cargo test -p executive -- impl::goal::retry`; expect PASS.
- [ ] Commit `feat(executive): define bounded goal retry policy`.

## 9. Task 8 — Implement AttemptCoordinator

**Files:**

- Create: `crates/executive/src/impl/goal/attempt_coordinator.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Test: `crates/executive/tests/attempt_coordinator.rs`

- [ ] Inject ObjectiveStore, SubAgentSpawner/RuntimeRegistry boundary, Clock, and cancellation.
- [ ] One call performs exactly one attempt:

```text
load Goal snapshot
check state/version and reserve Goal budget
persist Running attempt
spawn selected runtime once
await structured RuntimeResult/RuntimeFailure
persist terminal attempt and settle/revoke budget
evaluate RetryPolicy
persist next Goal state/wait reason/event
return; never loop into the next attempt
```

- [ ] Test success, retry with evidence, cancellation, timeout, missing runtime before attempt, provider transient backoff, reviewer escalation, budget settlement, persistence failure, and process cleanup.
- [ ] `SupervisorTree` may restart an unexpectedly crashed process only; normal runtime failures flow through AttemptCoordinator.
- [ ] Run `cargo test -p executive --test attempt_coordinator`; expect PASS.
- [ ] Commit `feat(executive): coordinate durable goal attempts`.

## 10. Task 9 — Build GoalFrame for every attempt

**Files:**

- Create: `crates/executive/src/impl/goal/frame.rs`
- Modify: `crates/executive/src/impl/goal/attempt_coordinator.rs`

- [ ] Define M3 GoalFrame with immutable original intent, desired state, constraints, acceptance criteria, current bounded task, recent attempt summaries, remaining budget, and retry evidence.
- [ ] Test original intent always appears, newest bounded attempt window, evidence truncation, deterministic ordering, and secret/token redaction.
- [ ] Render a stable prompt block; the runtime receives GoalFrame plus the selected task, not only free-form retry text.
- [ ] MemoryProjection integration remains an empty/default section until M8.
- [ ] Run `cargo test -p executive -- impl::goal::frame`; expect PASS.
- [ ] Commit `feat(executive): construct bounded goal frames`.

## 11. Task 10 — Integrate with Goal ticks and Telegram progress

**Files:**

- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/channel/router.rs`
- Test: `crates/executive/tests/goal_worker_flow.rs`

- [ ] Running Goal tick schedules exactly one AttemptCoordinator call.
- [ ] Persist progress events before sending Telegram notification.
- [ ] Test success notification, retry/backoff notification, escalation notification, AwaitingHuman, cancellation, duplicate tick/version conflict, and daemon restart between attempts.
- [ ] Do not expose raw provider errors or full tool output in Telegram; send bounded summaries with Goal/attempt IDs.
- [ ] Run `cargo test -p executive --test goal_worker_flow`; expect PASS.
- [ ] Commit `feat(executive): run supervised goal worker attempts`.

## 12. Task 11 — M3 release audit

- [ ] Run:

```bash
cargo fmt --all -- --check
cargo test -p fabric -- types::attempt
cargo test -p executive --test runtime_registry
cargo test -p executive -- impl::runtime::provider_worker
cargo test -p executive -- impl::goal::attempt
cargo test -p executive -- impl::goal::retry
cargo test -p executive --test attempt_coordinator
cargo test -p executive --test goal_worker_flow
cargo test --workspace
cargo build --workspace
```

- [ ] Prove two concurrent agents use distinct runtimes.
- [ ] Prove every runtime call has one durable attempt and budget settlement.
- [ ] Prove retry is bounded and third repeated failure escalates to a distinct runtime ID.
- [ ] Prove SupervisorTree contains no provider/model routing logic.
- [ ] Prove restart never silently reruns a stale attempt.

## 13. DeepSeek batches

1. Tasks 1–3: contracts and registry.
2. Tasks 4–5: provider runtime/config.
3. Task 6: persistence.
4. Tasks 7–8: policy/coordinator.
5. Tasks 9–11: frame/integration/audit.

Guardrails:

```text
Do not remove the legacy SubAgentRuntime::run path.
Do not put provider selection in SupervisorTree.
Do not start a second attempt in the same coordinator call.
Do not hard-code DeepSeek transport behavior.
Do not persist unbounded output or secrets.
Stop after each batch and report tests, exit codes, changed files, and failures.
```
