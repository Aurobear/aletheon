# E03 Governed Capability Invoker Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route CLI exec and daemon tool calls through one Executive-owned, fail-closed `CapabilityInvoker` and remove their manual admit/execute/settle code.

**Architecture:** Executive bootstrap constructs one `DefaultCapabilityInvoker<CorpusToolExecutor>` and decorates it with `GovernedCapabilityInvoker`, which attaches trusted turn policy and runs Dasein/approval review before delegation. Both front ends receive only `Arc<dyn CapabilityInvoker>`; neither can access registry, runner, admission, or settlement directly.

**Tech Stack:** Rust, Tokio, Kernel capability admission, Corpus adapter, Executive services

**Prerequisites:** E02 tests pass; `CorpusToolExecutor` is exported from Corpus; E01 baseline is green.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:962-979`, `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1176-1190`.

---

## Resolved design conflicts

| Earlier statement | Code reality | Resolution |
|---|---|---|
| Admission example used `scope` and String capability | `AdmissionRequest` requires `requested_scope`, `CapabilityId`, `action`, and `input_summary`: `crates/fabric/src/types/admission.rs:148-162` | Map every exact field from E02 call/authority |
| One `Arc` must be pointer-identical across daemon and exec | They have different composition roots: `exec_session.rs:91-174`, `daemon_turn/orchestrator.rs:63-115` | Require one factory and behavioral parity, not cross-process pointer identity |
| Existing Dasein capability reviewer could be adapted directly | Current review is a turn-level `sf_review` block: `turn_pipeline.rs:125-165` | Define an Executive authority port and explicit daemon/exec adapters |
| Cancellation could be handled by a synchronous drop guard | `AdmissionController::revoke` is async: `crates/fabric/src/include/admission.rs:40-45` | Kernel invocation owns an async select and performs exactly one settle-or-revoke transition |

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
let admission = AdmissionRequest {
    operation_id: request.call.operation_id,
    process_id: request.call.process_id,
    principal: request.authority.principal.clone(),
    capability: CapabilityId(request.call.name.clone()),
    action: request.authority.action.clone(),
    input_summary: format!("{:?}", request.call.input).chars().take(200).collect(),
    risk: request.authority.risk,
    requested_scope: request.authority.requested_scope.clone(),
    budget: request.authority.budget.clone(),
    lease: request.authority.lease.clone(),
    sandbox: request.authority.sandbox,
};
```

- [ ] Run the exact test; expected PASS. Authority cannot be absent because E02 makes it a required field; add a compile-fail doctest showing `CapabilityRequest { call }` is rejected.

### Task 2: Add the governed Executive decorator

- [ ] In `governed_capability_path.rs`, use a recording inner invoker and fake reviewer. Assert ordering `review -> inner`, authorized request construction, and that rejection leaves inner call count at zero.
- [ ] Run `cargo test -p executive --test governed_capability_path`; expected FAIL: type absent.
- [ ] Create:

```rust
#[async_trait]
pub trait TurnAuthorityProvider: Send + Sync {
    async fn authorize(&self, call: &CapabilityCall) -> anyhow::Result<AuthorizedInvocation>;
}

pub struct AuthorizedInvocation {
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

pub struct GovernedCapabilityInvoker {
    inner: Arc<dyn CapabilityInvoker>,
    authority: Arc<dyn TurnAuthorityProvider>,
}
```

Its Cognit-facing `invoke(call: CapabilityCall)` asks `TurnAuthorityProvider::authorize(&call)` for trusted authority plus the current operation cancellation token, constructs `CapabilityRequest { call, authority, control }`, then delegates once. The daemon adapter maps the existing `sf_review` verdict and request context (`turn_pipeline.rs:125-165`); exec maps `TerminalApprovalGate`, configured sandbox preference, session, and working directory (`exec_session.rs:121-159`). Review error, denial, or confirmation failure returns a structured error before Kernel admission.
- [ ] Run the focused test; expected PASS for approve/reject/order cases.

### Task 3: Construct one production graph

- [ ] Add a bootstrap test that obtains exec and daemon `Arc<dyn CapabilityInvoker>` handles and asserts both adapters were produced by the same `CapabilityRuntimeFactory` and share the same recording admission/executor Arcs.
- [ ] Run it; expected FAIL because no common runtime factory exists.
- [ ] At the existing Executive composition root, construct in this order:

```text
shared ToolRegistry + ToolRunnerWithGuard
 -> CorpusToolExecutor
 -> DefaultCapabilityInvoker(admission, executor)
 -> GovernedCapabilityInvoker(authority provider)
 -> Arc<dyn CapabilityInvoker>
```

Add `CapabilityRuntimeFactory::build(context) -> Arc<dyn TurnCapabilityInvoker>` and use it in both composition roots. Daemon and exec are separate application lifetimes, so pointer equality between modes is not required; identical factory composition and parity tests are the invariant. Do not expose the inner Kernel invoker.
- [ ] Run the bootstrap test; expected PASS.

### Task 4: Migrate exec and daemon and prove parity

- [ ] Add a table-driven integration test invoking a counting tool through exec and daemon. Assert equal output/error classification, recorder counts `admit=1`, `execute=1`, `settle=1`, `revoke=0`, and exactly one durable E02 audit row whose ID equals `CapabilityResult.audit_id` for each mode.
- [ ] Run it; expected FAIL because manual paths do not share the recorder.
- [ ] Replace exec lines 218-316 and daemon lines 392-452 with `governed.invoke(call).await`; keep only event projection outside the invoker. Delete builder fields for direct admission, registry, and runner access when no remaining consumer exists.
- [ ] Run the parity test; expected PASS with all counts exactly one.

### Task 5: Cancellation, sandbox, and settlement failure

- [ ] Add three tests: cancelled execution revokes once and never settles, required sandbox unavailable never executes, settlement failure returns an error result while preserving the permit/audit IDs.
- [ ] Run them; expected FAIL on at least the cancellation/settlement assertions.
- [ ] Use E02's non-serialized `InvocationControl` and add a Kernel-local `PermitDisposition` state. After admission, use `tokio::select!` between `executor.execute_with_permit(...)` and `request.control.cancel.cancelled()`. Execution completion calls `settle` once; cancellation calls `revoke(permit.id, RevokeReason::OperationCancelled)` once. Timeout is converted to cancellation by S02. Never settle or revoke in front ends.
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
