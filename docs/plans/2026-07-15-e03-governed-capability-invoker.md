# E03 Governed Capability Invoker Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route CLI exec and daemon tool calls through one Executive-owned, fail-closed `CapabilityInvoker` and remove their manual admit/execute/settle code.

**Architecture:** Executive bootstrap constructs one `DefaultCapabilityInvoker<CorpusToolExecutor>` and decorates it with `GovernedCapabilityInvoker`, which attaches trusted turn policy and runs Dasein/approval review before delegation. Both front ends receive only `Arc<dyn CapabilityInvoker>`; neither can access registry, runner, admission, or settlement directly.

**Tech Stack:** Rust, Tokio, Kernel capability admission, Corpus adapter, Executive services

**Prerequisites:** E02 tests pass; `CorpusToolExecutor` is exported from Corpus; E01 baseline is green.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:962-979`, `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1176-1190`.

---

## Anchors and invariants

- Hard-coded authority is in `crates/kernel/src/capability/mod.rs:67-86`.
- Exec duplicates admission/runtime/settlement at `crates/executive/src/service/exec_session.rs:218-316`.
- Daemon duplicates it at `crates/executive/src/service/turn_pipeline.rs:392-452`.
- Exactly one admission, execution, settlement, permit ID, and audit ID per call.
- Policy, sandbox, approval, cancellation, timeout, and settlement errors fail closed.
- Non-goals: unifying the surrounding turn lifecycle (S02) or changing tool-visible output.

```text
daemon ----\
             TurnServices -> GovernedCapabilityInvoker -> DefaultCapabilityInvoker -> Corpus
exec ------/                  review + trusted policy      admit/settle
```

## File map

- Modify: `crates/kernel/src/capability/mod.rs` — map all request policy fields.
- Create: `crates/executive/src/service/governed_capability.rs` — decorator and policy factory.
- Modify: `crates/executive/src/service/mod.rs` — internal export.
- Modify: `crates/executive/src/service/exec_session.rs` — invoker-only tool path.
- Modify: `crates/executive/src/service/turn_pipeline.rs` — invoker-only daemon path.
- Modify: Executive bootstrap module found by `rg -n 'ExecSessionBuilder|TurnPipeline::new' crates/executive crates/bin` — construct the shared graph.
- Create: `crates/executive/tests/governed_capability_path.rs` — parity and failure tests.

### Task 1: Remove Kernel hard-coded authority

- [ ] Add a Kernel test constructing a request with write risk, narrowed scope, budget, lease, and required sandbox; capture `AdmissionRequest` in a fake controller and assert exact equality for every field.
- [ ] Run `cargo test -p aletheon-kernel capability_request_maps_application_policy`; expected FAIL because current code emits read-only/default values.
- [ ] Replace lines 72-86 mapping with:

```rust
let policy = request.policy.as_ref().ok_or_else(|| anyhow!("missing application capability policy"))?;
let admission = AdmissionRequest {
    operation_id: request.operation_id,
    process_id: request.process_id,
    principal: policy.principal.clone(),
    capability: request.name.clone(),
    risk: policy.risk,
    scope: policy.scope.clone(),
    budget: policy.budget.clone(),
    lease: policy.lease.clone(),
    sandbox: policy.sandbox,
};
```

- [ ] Run the exact test; expected PASS. Add a missing-policy test; expected error before admission call count changes.

### Task 2: Add the governed Executive decorator

- [ ] In `governed_capability_path.rs`, use a recording inner invoker and fake reviewer. Assert ordering `review -> inner`, policy attachment, and that rejection leaves inner call count at zero.
- [ ] Run `cargo test -p executive --test governed_capability_path`; expected FAIL: type absent.
- [ ] Create:

```rust
pub struct TurnCapabilityContext {
    pub principal: PrincipalId,
    pub risk_for: Arc<dyn Fn(&str) -> RiskLevel + Send + Sync>,
    pub scope: CapabilityScope,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox_for: Arc<dyn Fn(&str) -> SandboxRequirement + Send + Sync>,
    pub session_id: String,
    pub working_dir: PathBuf,
}

pub struct GovernedCapabilityInvoker {
    inner: Arc<dyn CapabilityInvoker>,
    reviewer: Arc<dyn CapabilityReviewer>,
    context: TurnCapabilityContext,
}
```

Its `invoke` rejects pre-populated policy, asks the reviewer, creates `CapabilityPolicy` from trusted context, then delegates exactly once. Define `CapabilityReviewer` in this module as an async `review(&CapabilityRequest, &CapabilityPolicy) -> Result<()>` port and adapt the existing Dasein/approval service rather than copying its logic.
- [ ] Run the focused test; expected PASS for approve/reject/order cases.

### Task 3: Construct one production graph

- [ ] Add a bootstrap test that obtains exec and daemon `Arc<dyn CapabilityInvoker>` handles and asserts `Arc::ptr_eq` on the underlying shared governed invoker.
- [ ] Run it; expected FAIL because both paths build their own closures/runtime.
- [ ] At the existing Executive composition root, construct in this order:

```text
shared ToolRegistry + ToolRunnerWithGuard
 -> CorpusToolExecutor
 -> DefaultCapabilityInvoker(admission, audit, clock)
 -> GovernedCapabilityInvoker(reviewer, trusted turn context)
 -> Arc<dyn CapabilityInvoker>
```

Pass the final trait object into both builders. Do not expose the inner invoker from a public accessor.
- [ ] Run the bootstrap test; expected PASS.

### Task 4: Migrate exec and daemon and prove parity

- [ ] Add a table-driven integration test invoking a counting tool through exec and daemon. Assert equal output/error classification and recorder counts `admit=1`, `execute=1`, `settle=1`, `audit=1` for each mode.
- [ ] Run it; expected FAIL because manual paths do not share the recorder.
- [ ] Replace exec lines 218-316 and daemon lines 392-452 with `invoker.invoke(request).await`; keep only event projection outside the invoker. Delete builder fields for direct admission, registry, and runner access when no remaining consumer exists.
- [ ] Run the parity test; expected PASS with all counts exactly one.

### Task 5: Cancellation, sandbox, and settlement failure

- [ ] Add three tests: cancelled execution settles once, required sandbox unavailable never executes, settlement failure returns an error result and emits an audit failure.
- [ ] Run them; expected FAIL on at least the cancellation/settlement assertions.
- [ ] Add one drop-safe reservation guard in `DefaultCapabilityInvoker`: transition it from `Reserved` to `Settled` atomically, and settle cancellation/error in the same owner. Never settle in front ends.
- [ ] Run `cargo test -p executive --test governed_capability_path && cargo test -p executive --test capability_invoker`; expected PASS.

### Task 6: Remove bypass access and commit

- [ ] Make registry lookup and raw runner entry points `pub(crate)` where workspace consumers are gone; update E01 allowlist by deleting only resolved findings.
- [ ] Run `cargo fmt --all -- --check && cargo test -p aletheon-kernel && cargo test -p corpus --test capability_executor && cargo test -p executive --test governed_capability_path && cargo test -p executive --test turn_service_equivalence && bash scripts/architecture-check.sh`.
- [ ] Expected: all exit 0 and resolved manual-path allowlist lines disappear.
- [ ] Commit:

```text
feat(executive): converge tool calls on governed capability path

Exec and daemon performed their own admission, execution, and settlement, which
made policy and cancellation behavior diverge. Build one governed invoker and
expose only that contract to both front ends.

- map application authority without hard-coded policy
- order review before admission and guarded execution
- settle and audit every terminal path exactly once
```

## Compatibility deletion gate and evidence

Delete the temporary front-end adapter constructors once S02 routes both modes through `TurnCoordinator`. No production direct execution remains outside Corpus; E01 enforces this continuously.

- [ ] Exec and daemon parity assertions pass.
- [ ] Required sandbox, rejection, timeout, cancellation, and settlement failure are fail-closed.
- [ ] Each call has one permit ID, audit ID, and settlement.
- [ ] E01 shows no new bypass.
