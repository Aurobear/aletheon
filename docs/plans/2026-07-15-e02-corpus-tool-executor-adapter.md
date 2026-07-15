# E02 Corpus ToolExecutor Adapter Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put Corpus guarded tool execution behind Kernel `ToolExecutor` without allowing Cognit to invent principal, risk, scope, budget, lease, or sandbox authority.

**Architecture:** Split the current request into an untrusted `CapabilityCall` emitted by Cognit and a runtime-only authorized `CapabilityRequest { call, authority, control }` accepted by Kernel. Executive performs the only conversion in E03. Corpus implements the existing infallible `ToolExecutor` contract by converting lookup, permit, sandbox, and runner failures into structured error results.

**Tech Stack:** Rust, async-trait, Tokio, Fabric admission contracts, Corpus `ToolRegistry` and `ToolRunnerWithGuard`

**Prerequisites:** E01 passes with no new architecture findings.

**Source requirements:** Capability adapter and application authority at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:962-966`; fail-closed execution at `:969-979`; first vertical slice at `:1176-1183`.

---

## Resolved design conflicts

| Earlier plan statement | Code reality | Resolution in this plan |
|---|---|---|
| Adapter tests expected a Rust error return | `ToolExecutor::execute_with_permit` returns `CapabilityResult`, not `Result`: `crates/kernel/src/capability/mod.rs:41-50` | Assert `is_error`, output, usage, and audit fields |
| Cognit creates an authorized-shaped request without trusted authority | `TurnServices::invoke` is Cognit-facing: `crates/fabric/src/include/turn.rs:57-75` | Introduce separate untrusted `CapabilityCall`; authorized request has no optional authority |
| Permit capability equals a String | Permit uses `CapabilityId`: `crates/fabric/src/types/admission.rs:169-179` | Compare with `CapabilityId(request.call.name.clone())` |
| Runner call used owned values | Runner accepts `&dyn Tool`, `&ToolContext`: `crates/corpus/src/security/runner.rs:169-175` | Clone registry `Arc`, drop registry guard, pass borrowed values |
| Policy derived `Eq` | Budget/lease requests lack `Eq`: `crates/fabric/src/types/admission.rs:110-142` | Authority derives serialization and `Clone`, not equality |
| Calling the runner once means one side effect | Runner re-executes a structured tool after output-validation failure: `crates/corpus/src/security/runner.rs:385-399` | Remove execution retry; validate/sanitize the captured output or fail with the original execution count unchanged |

## Invariants and non-goals

- Cognit can describe a call but cannot supply authority.
- Kernel never admits a `CapabilityCall`; it admits only `CapabilityRequest`.
- Corpus checks permit binding and validity before registry lookup.
- `SandboxDecision::{Required,Unavailable,Failed}` produces an error without calling the tool, consistent with `ExecutionPermit::is_valid_at`: `crates/fabric/src/types/admission.rs:181-198`.
- The adapter invokes `ToolRunnerWithGuard::execute_tool` exactly once.
- Production wiring and manual-path deletion belong to E03.

```text
Cognit -> CapabilityCall -> Executive authorization (E03)
                              |
                              v
                     CapabilityRequest -> Kernel admit -> ExecutionPermit
                                                          |
                                                          v
                                                CorpusToolExecutor
                                                  | validate
                                                  | registry Arc clone
                                                  | guarded runner
                                                  ` CapabilityResult
```

## Exact file map

- Modify: `crates/fabric/src/include/turn.rs` — call/authority/request types and `TurnServices` signature.
- Modify: `crates/fabric/src/lib.rs` — explicit exports.
- Modify: `crates/cognit/src/harness/session.rs:92-104` — construct `CapabilityCall`.
- Modify: every `TurnServices` test fake returned by `rg -l 'impl TurnServices' crates` — signature-only migration.
- Create: `crates/corpus/src/tools/capability_executor.rs` — adapter.
- Modify: `crates/corpus/src/tools/tools/mod.rs` — module and re-export.
- Modify: `crates/fabric/src/security/audit.rs:13-27` — durable audit ID.
- Modify: `crates/corpus/src/security/runner.rs:169-470` — return the durable audit ID and propagate audit write failure.
- Create: `crates/corpus/tests/capability_executor.rs` — adapter contract suite.

### Task 1: Create the two-stage capability contract

- [ ] **Step 1: Write the failing boundary test** in `crates/fabric/tests/capability_contract.rs`:

```rust
use fabric::{CapabilityAuthority, CapabilityCall};

fn assert_serializable<T: serde::Serialize + serde::de::DeserializeOwned>() {}

#[test]
fn call_and_authority_are_serializable_wire_types() {
    assert_serializable::<CapabilityCall>();
    assert_serializable::<CapabilityAuthority>();
}
```

- [ ] **Step 2:** Run `cargo test -p fabric --test capability_contract`; expected FAIL with unresolved imports.

- [ ] **Step 3: Replace the current single type** in `crates/fabric/src/include/turn.rs` with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCall {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub name: String,
    pub input: serde_json::Value,
    pub call_id: String,
    pub deadline: Option<MonoDeadlineMillis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityAuthority {
    pub principal: PrincipalId,
    pub action: String,
    pub requested_scope: CapabilityScope,
    pub risk: RiskLevel,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
    pub session_id: String,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CapabilityRequest {
    pub call: CapabilityCall,
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

#[derive(Debug, Clone)]
pub struct InvocationControl {
    pub cancel: tokio_util::sync::CancellationToken,
}
```

Use imports from `crate::types::admission`, `operation`, and `std::path::PathBuf`. Re-export all four types from `fabric::lib`. Only the call and authority are serializable; the authorized request is runtime-only because its cancellation token is not a wire artifact. Fabric already depends on `tokio-util` at `crates/fabric/Cargo.toml:21`.

- [ ] **Step 4: Change the cognitive port** to `async fn invoke(&self, call: CapabilityCall) -> CapabilityResult`; update `StubTurnServices`, Cognit lines 93-100, and test fakes without adding defaults for authority.

- [ ] **Step 5:** Run `cargo test -p fabric --test capability_contract && cargo test -p cognit`; expected PASS.

### Task 2: Add deterministic adapter fixtures

- [ ] **Step 1:** In `crates/corpus/tests/capability_executor.rs`, define `CountingTool` with an `AtomicUsize`, register `Arc::new(tool)` through `fabric::Registry`, construct `ToolRunnerWithGuard` with test audit path and `TestClock`, and create matching call/authority/permit fixtures.

- [ ] **Step 2: Add the first failing test**:

```rust
#[tokio::test]
async fn mismatched_permit_fails_before_tool_lookup() {
    let (executor, request, mut permit, calls) = fixture();
    permit.operation_id = fabric::OperationId::new();
    let result = executor.execute_with_permit(&request, &permit).await;
    assert!(result.is_error);
    assert!(result.output.contains("permit does not bind request"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(result.usage.permit_id, permit.id);
}
```

- [ ] **Step 3:** Run `cargo test -p corpus --test capability_executor mismatched_permit_fails_before_tool_lookup -- --exact`; expected FAIL because the adapter module does not exist.

### Task 3: Implement permit validation and structured errors

- [ ] **Step 1: Create** `crates/corpus/src/tools/capability_executor.rs` with:

```rust
pub struct CorpusToolExecutor {
    registry: Arc<tokio::sync::RwLock<ToolRegistry>>,
    runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
    clock: Arc<dyn Clock>,
}

fn validate(req: &CapabilityRequest, permit: &ExecutionPermit, now: MonoTime) -> anyhow::Result<()> {
    anyhow::ensure!(
        permit.operation_id == req.call.operation_id
            && permit.process_id == req.call.process_id
            && permit.capability == CapabilityId(req.call.name.clone()),
        "permit does not bind request"
    );
    anyhow::ensure!(permit.is_valid_at(now), "permit expired or sandbox unavailable");
    Ok(())
}
```

- [ ] **Step 2:** Implement `error_result(call_id, permit_id, message)` returning `CapabilityResult` with `is_error=true`, zero counters, `usage.permit_id`, `exit_code=Some(1)`, and `audit_id=Some(AuditEventId::new())`.

- [ ] **Step 3:** Implement `ToolExecutor`; on validation error return `error_result` before acquiring the registry.

- [ ] **Step 4:** Run the exact mismatch test; expected PASS.

### Task 4: Make guarded runtime audit identity observable and fail-closed

- [ ] Add `pub audit_id: AuditEventId` to `AuditRecord` and update its fixtures. Generate the ID before execution so allow, deny, approval, loop-block, and tool-error records all have one identity.
- [ ] Add this result type beside `ToolError`:

```rust
pub struct GuardedToolExecution {
    pub result: Result<ToolResult, ToolError>,
    pub audit_id: AuditEventId,
}
```

- [ ] Change `AuditLogger`'s channel item to `(AuditRecord, oneshot::Sender<anyhow::Result<()>>)`. The writer acknowledges only after `writeln!` succeeds; `AuditLogger::log` awaits that acknowledgement. Keep `log_sync` best-effort only for non-capability legacy callers and forbid it in the adapter with E01. Change runner `log_audit` to accept the ID and return `anyhow::Result<()>`; add `ToolError::AuditFailed(String)` and make an audit persistence failure override the tool outcome.
- [ ] Change `execute_tool` to return `GuardedToolExecution`. Update current callers found by `rg -n 'execute_tool\(' crates --glob '*.rs'` mechanically to read `.result`; E02 adapter additionally reads `.audit_id`.
- [ ] Replace the output guardrail retry loop at `runner.rs:385-399`: execute the tool once, call guardrail validation once, and on rejection return `ToolError::OutputRejected` after auditing the captured result. Add a counting side-effect test whose guardrail rejects output and assert the tool count remains one.
- [ ] Add runner tests `success_returns_durable_audit_id` and `unwritable_audit_path_fails_execution`; parse the JSONL record and assert its `audit_id` equals the returned ID, then use a directory as the audit file path and assert `ToolError::AuditFailed`.
- [ ] Run `cargo test -p corpus security::runner`; expected PASS with matching IDs and a structured audit failure.

### Task 5: Execute through the guarded runtime exactly once

- [ ] **Step 1: Add tests** `guarded_tool_executes_once` and `missing_tool_is_structured_error`. The success test asserts call count 1, content, `is_error=false`, permit ID, output byte count, wall time from `ToolResultMeta`, and audit ID. The missing test asserts count 0 and message `tool not found: counting_tool`.

- [ ] **Step 2:** Run both exact tests; expected FAIL because only validation exists.

- [ ] **Step 3: Implement this exact lock/order sequence**:

```rust
let tool = {
    let registry = self.registry.read().await;
    registry.get(&request.call.name).cloned()
};
let Some(tool) = tool else { return error_result(/* tool not found */); };
let context = ToolContext {
    working_dir: request.authority.working_dir.clone(),
    session_id: request.authority.session_id.clone(),
    clock: self.clock.clone(),
};
let started = self.clock.mono_now();
let tool_result = self.runner.lock().await.execute_tool(
    tool.as_ref(),
    request.call.input.clone(),
    &context,
    &request.call.operation_id.0.to_string(),
).await;
```

Read `GuardedToolExecution.audit_id`. Convert its `Ok(ToolResult)` using `content`, `is_error`, and `metadata.execution_time_ms`; convert `Err(ToolError)` into a structured error with the same audit ID. Use measured monotonic elapsed time only when metadata is zero. Do not synthesize a second audit ID in the adapter.

- [ ] **Step 4:** Run `cargo test -p corpus --test capability_executor`; expected PASS.

### Task 6: Prove fail-closed sandbox and lock behavior

- [ ] Add table rows for expired, `Required`, `Unavailable`, and `Failed` permits; each must return an error and keep tool count at zero.
- [ ] Add a blocking tool test: while it awaits a notify, acquire a registry write lock under a 100 ms Tokio timeout; this proves no registry guard crosses tool await.
- [ ] Run `cargo test -p corpus --test capability_executor`; expected PASS with no timeout.

### Task 7: Verify and commit

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p fabric --test capability_contract && cargo test -p cognit && cargo test -p corpus --test capability_executor`.
- [ ] Run `cargo check --workspace --all-targets && bash scripts/architecture-check.sh`.
- [ ] Expected: every command exits 0; E01 reports no new direct-tool bypass.
- [ ] Inspect the staged diff and commit:

```text
feat(corpus): adapt guarded tools to authorized capabilities

Cognit call descriptions and application authority were conflated, while Kernel
admission could not reach Corpus without bypassing its executor port. Separate
untrusted calls from authorized requests and add a fail-closed Corpus adapter.

- make Executive the only call-to-authority conversion boundary
- validate permit identity and sandbox state before lookup
- execute once through ToolRunner and return settled usage inputs
```

## Compatibility and completion evidence

`CapabilityCall` is permanent at the Cognit boundary; `CapabilityRequest` is permanent at the Kernel boundary. E03 deletes every production conversion except the governed Executive invoker. E03 also makes registry and raw runner access inaccessible outside Corpus after their callers migrate.

- [ ] Untrusted Cognit code cannot construct an authorized request through `TurnServices`.
- [ ] Mismatch, expiry, unavailable sandbox, and missing tool execute zero times.
- [ ] Successful execution count is one and result carries the permit ID and audit ID.
- [ ] Registry locks do not cross tool execution await.
- [ ] Focused, workspace, and E01 gates pass.
