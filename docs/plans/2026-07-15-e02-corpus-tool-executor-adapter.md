# E02 Corpus ToolExecutor Adapter Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute Corpus tools behind Kernel `ToolExecutor` with permit validation, runtime guardrails, and exactly one usage report.

**Architecture:** Add application-supplied capability policy to the Fabric request, then implement a Corpus adapter over `ToolRegistry` and `ToolRunnerWithGuard`. Cognit creates an untrusted request without policy; Executive will attach trusted policy in E03. The adapter fails closed when policy or permit context is missing or inconsistent.

**Tech Stack:** Rust, async-trait, Tokio, Fabric admission contracts, Corpus ToolRunner

**Prerequisites:** E01 passes with no new architecture findings.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:958-979`, `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1176-1183`.

---

## Anchors, boundary, and file map

- `CapabilityRequest` lacks policy context: `crates/fabric/src/include/turn.rs:34-41`.
- `ToolExecutor` is the Kernel execution port: `crates/kernel/src/capability/mod.rs:27-50`.
- Tool lookup and guarded execution exist at `crates/corpus/src/tools/tools/registry.rs:8-107` and `crates/corpus/src/security/runner.rs:169-410`.
- Cognit constructs requests at `crates/cognit/src/harness/session.rs:93-100`.
- Non-goals: production wiring, daemon/exec deletion, or making Registry public.

```text
Cognit request(policy=None) -> Executive enrichment(E03) -> DefaultCapabilityInvoker
 -> CorpusToolExecutor -> permit validation -> ToolRunnerWithGuard -> UsageReport
```

- Modify: `crates/fabric/src/include/turn.rs` — `CapabilityPolicy` and request field.
- Modify: `crates/fabric/src/lib.rs` — re-export policy.
- Modify: `crates/cognit/src/harness/session.rs` — explicitly untrusted request.
- Create: `crates/corpus/src/tools/capability_executor.rs` — adapter.
- Modify: `crates/corpus/src/tools/mod.rs` — export adapter.
- Create: `crates/corpus/tests/capability_executor.rs` — contract tests.

### Task 1: Add the application policy contract

- [ ] **Step 1: Add a Fabric serialization test** in `crates/fabric/tests/protocol_e2e.rs`:

```rust
#[test]
fn capability_policy_round_trips() {
    let policy = fabric::CapabilityPolicy {
        principal: fabric::PrincipalId("user:test".into()),
        risk: fabric::RiskLevel::ReadOnly,
        scope: fabric::CapabilityScope::default(),
        budget: None,
        lease: None,
        sandbox: fabric::SandboxRequirement::Required,
        session_id: "s1".into(),
        working_dir: std::path::PathBuf::from("/tmp/work"),
    };
    let json = serde_json::to_string(&policy).unwrap();
    assert_eq!(serde_json::from_str::<fabric::CapabilityPolicy>(&json).unwrap(), policy);
}
```

- [ ] **Step 2:** Run `cargo test -p fabric --test protocol_e2e capability_policy_round_trips -- --exact`; expected FAIL: unresolved `CapabilityPolicy`.

- [ ] **Step 3: Add the exact type and field** to `include/turn.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPolicy {
    pub principal: PrincipalId,
    pub risk: RiskLevel,
    pub scope: CapabilityScope,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
    pub session_id: String,
    pub working_dir: PathBuf,
}
```

Add `pub policy: Option<CapabilityPolicy>` to `CapabilityRequest`, import the named Fabric types, re-export it from `fabric::lib`, and set `policy: None` in Cognit. This `None` is an explicit untrusted boundary, not a permissive default.

- [ ] **Step 4:** Run `cargo test -p fabric --test protocol_e2e capability_policy_round_trips -- --exact && cargo test -p cognit`; expected PASS.

### Task 2: Prove fail-closed adapter behavior

- [ ] **Step 1: Create** `crates/corpus/tests/capability_executor.rs` with a fixture using `TestClock`, an empty registry, and a permit whose operation/process/capability match the request. Add tests:

```rust
#[tokio::test]
async fn rejects_request_without_application_policy() {
    let (executor, request, permit) = fixture();
    let err = executor.execute_with_permit(&request, &permit).await.unwrap_err();
    assert!(err.to_string().contains("missing application capability policy"));
}

#[tokio::test]
async fn rejects_expired_or_mismatched_permit() {
    let (executor, request, mut permit) = fixture_with_policy();
    permit.operation_id = fabric::OperationId::new();
    let err = executor.execute_with_permit(&request, &permit).await.unwrap_err();
    assert!(err.to_string().contains("permit does not bind request"));
}
```

- [ ] **Step 2:** Run `cargo test -p corpus --test capability_executor`; expected FAIL: module/type absent.

- [ ] **Step 3: Create the adapter skeleton**:

```rust
pub struct CorpusToolExecutor {
    registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
    runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
    clock: Arc<dyn Clock>,
}

fn validate(request: &CapabilityRequest, permit: &ExecutionPermit, now: MonoTime) -> Result<&CapabilityPolicy> {
    let policy = request.policy.as_ref().ok_or_else(|| anyhow!("missing application capability policy"))?;
    ensure!(permit.operation_id == request.operation_id
        && permit.process_id == request.process_id
        && permit.capability == request.name, "permit does not bind request");
    ensure!(permit.is_valid_at(now), "permit expired or revoked");
    Ok(policy)
}
```

Implement `ToolExecutor` and return before registry lookup on validation error.

- [ ] **Step 4:** Run `cargo test -p corpus --test capability_executor`; expected both rejection tests PASS.

### Task 3: Execute once through guarded Corpus runtime

- [ ] **Step 1:** Add `CountingTool` to the test, register it, invoke once, and assert output, count `1`, permit ID in the result metadata, and a non-empty usage report.
- [ ] **Step 2:** Run the exact test; expected FAIL because successful execution is not implemented.
- [ ] **Step 3: Implement the success path**:
  1. clone the `Arc<dyn Tool>` while holding the registry lock, then release it;
  2. build `ToolContext { working_dir, session_id, clock, .. }` only from `CapabilityPolicy`;
  3. call `runner.execute_tool(tool, request.input.clone(), context, request.operation_id.to_string())` exactly once;
  4. convert guarded output/error to `CapabilityResult`;
  5. report measured wall duration and output bytes in one `UsageReport`.

Use the repository’s existing concrete `ToolContext`, `ToolExecutionResult`, and `UsageReport` field names as verified immediately before editing; if their signatures differ from this plan, update this plan in the same documentation commit before implementation.

- [ ] **Step 4:** Run `cargo test -p corpus --test capability_executor executes_guarded_tool_once -- --exact`; expected PASS and count `1`.

### Task 4: Verify sandbox failure and concurrency safety

- [ ] Add tests `required_sandbox_without_backend_fails_closed` and `registry_lock_is_not_held_during_tool_await` (the latter uses a blocking tool plus a timed registry read).
- [ ] Run both tests; expected FAIL before enforcement.
- [ ] Pass `policy.sandbox` into the existing ToolRunner sandbox decision and reject `Required` when no backend is configured. Ensure all mutex guards are dropped before `.await`.
- [ ] Run `cargo test -p corpus --test capability_executor`; expected PASS without timeout.

### Task 5: Scoped verification and commit

- [ ] Run: `cargo fmt --all -- --check && cargo test -p fabric --test protocol_e2e && cargo test -p cognit && cargo test -p corpus --test capability_executor && bash scripts/architecture-check.sh`.
- [ ] Expected: all exit 0; architecture allowlist has no new direct execution finding.
- [ ] Inspect the staged diff and commit:

```text
feat(corpus): adapt guarded tools to capability execution

Kernel admission could not reach the real Corpus runtime without bypassing the
ToolExecutor boundary. Add application policy context and a fail-closed adapter
that validates permits before guarded tool execution.

- carry trusted policy context separately from Cognit requests
- reject missing, expired, and mismatched authority
- execute once through ToolRunner and report usage
```

## Deletion gate and evidence

The `policy: Option<_>` compatibility shape is deleted in S02 when every CognitiveSession enters a coordinator that can construct a trusted request; it becomes a required internal governed request. Raw Registry/Runner access remains crate-private after E03 removes external users.

- [ ] Missing policy, mismatched permit, expired permit, unavailable required sandbox all fail closed.
- [ ] Successful tool count is exactly one and produces one usage report.
- [ ] No mutex guard crosses a tool future.
- [ ] E01 remains green.
