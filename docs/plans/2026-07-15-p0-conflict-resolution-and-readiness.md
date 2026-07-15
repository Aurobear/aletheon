# P0 Conflict Resolution and Implementation Readiness

> **Status:** Normative companion for E01–E03 and S01–S02
>
> **Baseline inspected:** `65f74981`
>
> **Purpose:** Resolve requirement/code/plan conflicts before implementation workers begin.

## 1. Requirement coverage matrix

| Requirement | Source anchor | Owning plan/task | Completion evidence |
|---|---|---|---|
| Architecture checker | `architecture-coupling...:941` | E01 Tasks 1–4 | fixture, local command, CI job |
| Allowed dependency graph | `:942` | E01 Task 3 | Cargo metadata edge baseline |
| Legacy/bypass inventory | `:943-949` | E01 Tasks 1–2 | normalized shrink-only baseline |
| Correct stale architecture claims | `:950` | E01 Task 4 Step 5 | documentation search reviewed against generated inventories |
| Corpus Kernel adapter | `:964-965` | E02 Tasks 2–6 | guarded adapter integration suite |
| Application-supplied authority | `:966` | E02 Task 1; E03 Tasks 1–2 | separate call/request types and captured admission request |
| One governed Executive path | `:967-971` | E03 Tasks 2–6 | factory composition and daemon/exec parity |
| Permit/audit/settlement invariants | `:973-979` | E02 Tasks 3–6; E03 Tasks 4–5 | counting controller/executor tests |
| Versioned lifecycle records | `:985` | S01 Task 1 | JSON fixtures |
| Generated schema and client artifacts | `:986` | S01 Task 2 | byte-identical schema and Interact snapshots |
| Canonical append store | `:987` | S01 Tasks 3–6 | idempotency/conflict/reopen tests |
| One coordinator and factory | `:988-989` | S02 Tasks 1–6 | daemon/exec equivalence |
| Mode policies | `:990-996` | S02 Task 1 | policy field coverage test |
| Remove duplicate ReAct state | `:997` | S02 Task 8 | architecture and regression tests |
| Resume/fork/interrupt/replay | `:998` | S02 Task 7 | four command integration tests |
| Correct Turn identity | `:1002-1004` | S02 Tasks 2–6 | lifecycle matrix and restart replay |
| Typed Interact fields | `:1005` | S01 Task 2; S02 Task 7 | typed request/notification snapshots |
| History/memory/trace separation | `:1006` | S01 Tasks 4–6 | table-isolation assertions |

No Phase 0–2 item is intentionally deferred beyond S02.

## 2. Authoritative conflict decisions

### 2.1 Cognit call versus application authority

```text
CapabilityCall                 CapabilityRequest
---------------                -----------------
operation/process              call: CapabilityCall
tool name/input                authority:
call ID/deadline                 principal/action/risk
                                  scope/budget/lease/sandbox
                                  session/working directory
       |                                  ^
       `---- Cognit -> TurnServices ------| Executive only
```

The current `CapabilityRequest` is constructed inside Cognit at `crates/cognit/src/harness/session.rs:92-104`, but the source requires authority to come from the application. An optional policy would compile but remain forgeable or absent. E02 therefore creates two types; E03 owns the only production conversion.

### 2.2 Error contract

`ToolExecutor` is deliberately infallible at the Rust type level and returns structured `CapabilityResult`: `crates/kernel/src/capability/mod.rs:41-50`. Corpus adapter failures therefore set `is_error=true`; they do not return `anyhow::Error`. Private validation helpers may return `Result`, but the trait boundary converts every error.

### 2.3 Sandbox ownership

The current permit considers `Required`, `Unavailable`, and `Failed` invalid: `crates/fabric/src/types/admission.rs:181-198`. P0 preserves this fail-closed contract. Corpus owns runtime sandbox execution/guardrails, but it only executes when admission returns `NotApplicable` or `Passed`. Promoting a `Required` permit to `Passed` requires a future typed admission/sandbox handshake and is not silently invented here.

### 2.3.1 Side-effect cardinality

The current runner can call a structured tool again when output validation fails: `crates/corpus/src/security/runner.rs:385-399`. Output validation cannot undo a side effect, so retrying execution violates the one-permit/one-execution invariant. E02 removes execution retry and converts rejection into an audited error while preserving an execution count of one.

### 2.4 Cancellation disposition

`AdmissionController` distinguishes `settle` and `revoke`: `crates/fabric/src/include/admission.rs:33-45`. Kernel owns the permit after admission:

```text
admitted -> execution completed -> settle exactly once
         -> cancellation/timeout -> revoke exactly once
         -> settlement failure   -> structured error, never retry implicitly
```

Front ends never settle or revoke. S02 turns a deadline into cancellation; E03 performs the permit transition.

### 2.5 Operation lifecycle port gap

Fabric `OperationManager` lacks start/succeed/fail: `crates/fabric/src/include/process.rs:32-37`, while current `TurnService` calls concrete operation-table methods at `crates/executive/src/service/turn_service.rs:81-155`. S02 uses the concrete table already exposed by `ServicePorts` as a documented compatibility boundary. Expanding the port belongs to architecture Phase 3; pretending the current trait supports those methods would make P0 uncompilable.

### 2.6 “One invoker” across application modes

Exec constructs its runtime at `crates/executive/src/service/exec_session.rs:91-174`; daemon constructs another application lifetime at `crates/executive/src/service/daemon_turn/orchestrator.rs:63-115`. “One” means one composition factory and one authoritative chain per runtime, not pointer equality between separately started processes. Tests prove identical ordering and behavior with shared fakes.

## 3. Cross-plan signatures

These signatures are frozen for P0 implementation. Changing one requires editing every listed consumer in the same documentation revision.

```rust
// Fabric: Cognit-facing
async fn TurnServices::invoke(&self, call: CapabilityCall) -> CapabilityResult;

// Fabric: application-authorized
pub struct CapabilityRequest {
    pub call: CapabilityCall,
    pub authority: CapabilityAuthority,
    pub control: InvocationControl, // non-serialized CancellationToken wrapper
}

// Kernel: unchanged execution port
async fn ToolExecutor::execute_with_permit(
    &self,
    request: &CapabilityRequest,
    permit: &ExecutionPermit,
) -> CapabilityResult;

// Executive: only authorization conversion
async fn TurnAuthorityProvider::authorize(
    &self,
    call: &CapabilityCall,
) -> anyhow::Result<AuthorizedInvocation>; // authority + InvocationControl
```

`CapabilityCall` and lifecycle protocol records are serializable. `InvocationControl` and the authorized Kernel request are runtime-only because a cancellation token is not a wire artifact.

## 4. Implementation order and merge gates

```text
E01
 |
 +--> S01 -------------------+
 |                           |
 `--> E02 -> E03 ------------+--> S02
```

- E01 merges when the current baseline passes and a synthetic addition fails.
- E02 merges with no production caller; its adapter and contracts are isolated and tested.
- E03 merges only after exec and daemon parity plus cancellation/settlement tests pass.
- S01 merges as unused canonical contracts/store plus a legacy projection adapter.
- S02 merges only after full workspace tests, four lifecycle commands, restart replay, and architecture checks pass.

## 5. Worker stop conditions

An implementer must stop and revise the plan rather than guess when:

1. a referenced production symbol no longer exists on the implementation branch;
2. an existing field type differs from the frozen signature above;
3. a migration would expose raw Registry/Runner outside Corpus;
4. cancellation cannot prove exactly one settle-or-revoke disposition;
5. a lifecycle JSON change is not accompanied by schema and Interact snapshot updates;
6. a task requires a later plan to compile or keep the default test suite green.

## 6. Readiness checklist

- [ ] Every Phase 0–2 source item maps to a named task above.
- [ ] Every code claim has a current path/line anchor.
- [ ] E02/E03 use exact current admission and executor field names.
- [ ] S02 does not call methods absent from `OperationManager`.
- [ ] Schema/client/lifecycle command requirements are not deferred.
- [ ] Each plan has focused tests, workspace gates, and a commit boundary.
- [ ] Placeholder scan and `git diff --check` pass.

When all seven checks pass, P0 is ready for implementation in DAG order.
