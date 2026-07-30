# Coding Capability Evaluation Kernel Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect explicitly typed coding turns to an evidence-backed, durable evaluation receipt inside authoritative Turn settlement, beginning in shadow mode and supporting host-configured enforcement.

**Architecture:** Fabric owns versioned task/evaluation contracts, Executive issues contracts and settles evaluation, Cognit performs in-turn completeness checks, and Metacog validates evidence references and aggregates scores. The daemon persists contracts/evidence/receipts in SQLite before terminalizing the Turn, then exposes the same receipt to clients and non-authoritative core projections.

**Tech Stack:** Rust, Tokio, Serde/Schemars, SHA-256, rusqlite/WAL, existing Kernel operation tree, Fabric client/session protocols, Cognit completion gate, Metacog deterministic evaluator.

**Status:** Tasks 1-14 are an as-built baseline accepted on 2026-07-30. The two
L2 core-consumer closures were implemented on 2026-07-31 and passed their focused
tests; the repeated installed-runtime acceptance remains pending below.

---

## Requirement anchors

- Design specification: `docs/plans/2026-07-30-coding-capability-evaluation-kernel-design.md:1`.
- Explicit coding task recognition: design sections 2, 6, and 7.1.
- Authoritative settlement: design sections 5, 7.2, and 8.
- `coding-v2` evidence-backed scoring: design section 9.
- Shadow/enforce behavior: design sections 10 and 12.
- Persistence, projections, and installed acceptance: design sections 11, 13, and 15.

## File map

```text
crates/fabric/src/types/evaluation/
  mod.rs             IDs, shared exports, schema version
  contract.rs        TaskKind, mode, subject, host-issued contract
  evidence.rs        immutable evidence snapshot and validation
  receipt.rs         decision, receipt, and Session-safe receipt reference

crates/executive/src/application/evaluation/
  mod.rs             public application exports
  contract_issuer.rs host policy -> contract
  evidence_collector.rs Turn artifacts -> EvidenceItem snapshot
  coding_scorer.rs   coding-v2 dimension and gate rules
  policy.rs          report -> shadow/enforce decision and TurnStop
  service.rs         child operation, persistence, evaluation orchestration

crates/executive/src/adapters/evaluation/
  mod.rs             adapter exports
  sqlite_store.rs    WAL-backed evaluation repository

crates/executive/tests/
  evaluation_turn_settlement.rs authoritative integration matrix
  evaluation_protocol_e2e.rs    typed chat/queue/receipt replay
```

---

### Task 1: Add the Fabric evaluation ABI

**Files:**
- Create: `crates/fabric/src/types/evaluation/mod.rs`
- Create: `crates/fabric/src/types/evaluation/contract.rs`
- Create: `crates/fabric/src/types/evaluation/evidence.rs`
- Create: `crates/fabric/src/types/evaluation/receipt.rs`
- Modify: `crates/fabric/src/types/mod.rs:1-81`
- Modify: `crates/fabric/src/lib.rs:56-92`
- Test: inline unit tests in the four new files

- [x] **Step 1: Write failing round-trip and validation tests**

```rust
#[test]
fn coding_contract_round_trips_and_rejects_invalid_thresholds() {
    let contract = fixture_contract(EvaluationMode::Shadow);
    let json = serde_json::to_string(&contract).unwrap();
    assert_eq!(serde_json::from_str::<TaskEvaluationContract>(&json).unwrap(), contract);
    let mut invalid = contract;
    invalid.min_evidence_coverage_millis = 1_001;
    assert!(invalid.validate().is_err());
}

#[test]
fn evidence_snapshot_digest_is_order_stable_and_detects_payload_changes() {
    let first = fixture_evidence("e-1", serde_json::json!({"ok": true}));
    let snapshot = EvaluationEvidenceSnapshot::new("c-1".into(), vec![first.clone()]).unwrap();
    assert!(snapshot.validate().is_ok());
    let mut tampered = snapshot;
    tampered.evidence[0].payload = serde_json::json!({"ok": false});
    assert!(tampered.validate().is_err());
}
```

- [x] **Step 2: Run the Fabric library tests and observe missing types**

Run: `bash scripts/cargo-agent.sh test -p fabric --lib evaluation -- --nocapture`

Expected: FAIL because `types::evaluation` and its contracts do not exist.

- [x] **Step 3: Implement the versioned types**

Use newtype UUID IDs and these public shapes:

```rust
pub const EVALUATION_SCHEMA_V1: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind { Coding }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationMode { Shadow, Enforce }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluationSubject {
    Turn { turn_id: TurnId, operation_id: OperationId },
    GoalAttempt { goal_id: GoalId, attempt_id: AttemptId },
    AgentRun { agent_id: AgentId, operation_id: OperationId },
    CodingJob { job_id: CodingJobId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvaluationThresholds {
    pub min_score_millis: u32,
    pub min_evidence_coverage_millis: u16,
    pub min_confidence_millis: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvaluationReceiptRef {
    pub schema_version: u16,
    pub receipt_id: EvaluationReceiptId,
    pub contract_id: EvaluationContractId,
    pub subject: EvaluationSubject,
    pub decision: EvaluationDecision,
    pub weighted_total_millis: Option<u32>,
    pub evidence_coverage_millis: u16,
    pub confidence_millis: u16,
    pub failed_gates: Vec<String>,
    pub created_at_ms: i64,
}
```

`TaskEvaluationContract::validate` must enforce schema version 1, non-empty
issuer/rubric/objective reference, score `<= 100_000`, coverage/confidence
`<= 1_000`, non-empty required gates, and a Turn subject matching the request's
authoritative Turn/operation IDs.

`EvaluationEvidenceSnapshot::new` must sort by `EvidenceId`, reject duplicate
IDs, validate every payload digest, and compute SHA-256 over canonical serialized
ordered evidence. `EvaluationReceipt::validate` must verify schema/version,
contract/subject identity, snapshot digest, decision/mode compatibility, and
failed-gate/report consistency.

- [x] **Step 4: Export the module and rerun tests**

Run: `bash scripts/cargo-agent.sh test -p fabric --lib evaluation -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(fabric): add evaluation contracts and receipts`

---

### Task 2: Carry explicit task kind through client, queue, and Turn requests

**Files:**
- Modify: `crates/fabric/src/protocol/client.rs:9-106,311-348,986-1010`
- Modify: `crates/fabric/src/types/prompt_queue.rs:46-69`
- Modify: `crates/fabric/src/types/turn.rs:8-20`
- Modify: `crates/executive/src/application/session_input.rs:183-235`
- Modify: `crates/executive/src/adapters/session/prompt_queue_sqlite.rs`
- Test: `crates/fabric/src/protocol/client.rs` unit tests
- Test: `crates/executive/src/adapters/session/prompt_queue_sqlite.rs` unit tests

- [x] **Step 1: Add failing typed transport tests**

```rust
#[test]
fn chat_serializes_explicit_coding_task_kind() {
    let workspace = WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![]).unwrap();
    let request = ClientRpcRequest::chat_with_task_kind(
        "change code", None, &workspace, vec![], Some(TaskKind::Coding),
    ).to_json_rpc(Some(7)).unwrap();
    assert_eq!(request["params"]["task_kind"], "coding");
}

#[test]
fn chat_without_task_kind_omits_the_field() {
    let workspace = WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![]).unwrap();
    let request = ClientRpcRequest::chat("hello", &workspace).to_json_rpc(Some(8)).unwrap();
    assert!(request["params"].get("task_kind").is_none());
}
```

- [x] **Step 2: Run focused tests**

Run: `bash scripts/cargo-agent.sh test -p fabric protocol::client::request_tests -- --nocapture`

Expected: FAIL because `task_kind` and `chat_with_task_kind` do not exist.

- [x] **Step 3: Implement optional typed fields with compatibility defaults**

Add to `ChatParams`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub task_kind: Option<crate::TaskKind>,
```

Add `requested_task_kind: Option<TaskKind>` to `PromptEnvelope`; add
`#[serde(default)]` so old rows replay. Add both fields to `TurnRequest`:

```rust
#[serde(default)]
pub requested_task_kind: Option<crate::TaskKind>,
#[serde(default)]
pub evaluation_contract: Option<crate::TaskEvaluationContract>,
```

Extend queue enqueue methods to accept task kind in the same call that accepts
requirements, and persist it in the SQLite JSON/body representation. The
plain `enqueue` method passes `None`.

- [x] **Step 4: Run Fabric and prompt-queue tests**

Run: `bash scripts/cargo-agent.sh test -p fabric protocol::client::request_tests -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p executive prompt_queue_sqlite -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(protocol): carry explicit coding task kind`

---

### Task 3: Add evaluation operation/session contracts and configuration

**Files:**
- Modify: `crates/fabric/src/types/operation.rs:49-58`
- Modify: `crates/fabric/src/types/session.rs:11,107-146`
- Create: `crates/executive/src/composition/config/evaluation.rs`
- Modify: `crates/executive/src/composition/config/mod.rs:6-44,83-122`
- Test: `crates/executive/src/composition/config/evaluation.rs`
- Test: `crates/fabric/src/types/session.rs`

- [x] **Step 1: Add failing config and Session compatibility tests**

```rust
#[test]
fn evaluation_defaults_disabled_and_shadow() {
    let settings = EvaluationSettings::default();
    assert!(!settings.enabled);
    assert_eq!(settings.default_mode, EvaluationMode::Shadow);
    assert_eq!(settings.coding_rubric, "coding-v2");
    assert_eq!(settings.min_score_millis, 70_000);
}

#[test]
fn receipt_ref_round_trips_in_session_item() {
    let payload = ItemPayload::EvaluationReceiptRef { receipt: fixture_ref() };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(serde_json::from_str::<ItemPayload>(&json).unwrap(), payload);
}
```

- [x] **Step 2: Run tests and observe missing variants/settings**

Run: `bash scripts/cargo-agent.sh test -p executive evaluation_defaults_disabled_and_shadow -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p fabric receipt_ref_round_trips_in_session_item -- --nocapture`

Expected: FAIL.

- [x] **Step 3: Implement configuration and variants**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluationSettings {
    pub enabled: bool,
    pub default_mode: EvaluationMode,
    pub coding_rubric: String,
    pub min_score_millis: u32,
    pub min_evidence_coverage_millis: u16,
    pub min_confidence_millis: u16,
    pub max_evaluation_ms: u64,
}
```

Default to disabled/shadow/`coding-v2`/70_000/600/700/5_000 and provide a
`validate` method for the same numeric bounds as the contract. Add
`evaluation: EvaluationSettings` to `AppConfig`, `Evaluation` to
`OperationKind`, and `EvaluationReceiptRef` to `ItemPayload`. Increment
`SESSION_SCHEMA_VERSION` and preserve decoding of prior tagged payloads.

- [x] **Step 4: Run config/schema/Fabric tests**

Run: `bash scripts/cargo-agent.sh test -p executive composition::config -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p fabric --lib session -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(runtime): configure evaluation settlement`

---

### Task 4: Make Metacog validate real evidence coverage and confidence

**Files:**
- Modify: `crates/metacog/src/evaluation/engine.rs:40-205`
- Modify: `crates/metacog/src/evaluation/mod.rs:1-9`
- Test: inline tests in `crates/metacog/src/evaluation/engine.rs`

- [x] **Step 1: Add failing integrity tests**

```rust
#[test]
fn evidence_backed_evaluation_rejects_unknown_reference() {
    let input = fixture_scored_input(vec![EvidenceId("missing".into())]);
    let result = DeterministicEvaluator::new().evaluate_evidence_backed(
        &fixture_rubric(), input.0, input.1, &[], fixture_thresholds(),
    );
    assert!(matches!(result, Err(EvaluationError::UnknownEvidenceReference(_))));
}

#[test]
fn coverage_is_weight_based_and_unverified_evidence_cannot_score() {
    let evidence = vec![fixture_evidence("correctness", EvidenceTrust::Authoritative)];
    let report = evaluate_one_weighted_dimension(&evidence, 250_000).unwrap();
    assert_eq!(report.evidence_coverage_millis, 250);
    assert_eq!(report.confidence_millis, 1_000);
}
```

- [x] **Step 2: Run Metacog tests**

Run: `bash scripts/cargo-agent.sh test -p metacog evaluation::engine -- --nocapture`

Expected: FAIL because evidence-backed evaluation and errors do not exist.

- [x] **Step 3: Add `evaluate_evidence_backed` without breaking the legacy method**

The new method receives `&[EvidenceItem]` and `EvaluationThresholds`. It first
calls the existing structural/weight calculation, then verifies:

```rust
for dimension in report.dimensions.iter().filter(|d| matches!(d.value, DimensionValue::Scored(_))) {
    if dimension.evidence.is_empty() { return Err(EvaluationError::MissingEvidence(dimension.name.clone())); }
    for id in &dimension.evidence { resolve_supported_evidence(id, evidence)?; }
}
for gate in &report.gates {
    if gate.evidence.is_empty() { return Err(EvaluationError::MissingGateEvidence(gate.name.clone())); }
    for id in &gate.evidence { resolve_supported_evidence(id, evidence)?; }
}
```

Coverage is the sum of rubric weights for scored dimensions with valid evidence
divided by total rubric weight. Confidence is the weight-adjusted mean trust:
authoritative 1000, corroborated 700, unverified 0; unverified references return
`UnsupportedEvidenceTrust`. Eligibility additionally requires contract coverage
and confidence thresholds.

- [x] **Step 4: Run Metacog tests**

Run: `bash scripts/cargo-agent.sh test -p metacog evaluation::engine -- --nocapture`

Expected: PASS, including all existing deterministic evaluator tests.

- [x] **Step 5: Commit**

Commit subject: `feat(metacog): validate evidence-backed evaluations`

---

### Task 5: Implement `coding-v2`, evidence collection, and deterministic scoring

**Files:**
- Create: `crates/executive/src/application/evaluation/mod.rs`
- Create: `crates/executive/src/application/evaluation/contract_issuer.rs`
- Create: `crates/executive/src/application/evaluation/evidence_collector.rs`
- Create: `crates/executive/src/application/evaluation/coding_scorer.rs`
- Create: `crates/executive/src/application/evaluation/policy.rs`
- Modify: `crates/executive/src/application/mod.rs:1-67`
- Modify: `crates/executive/src/application/turn_diff_tracker.rs:7-77`
- Test: inline tests in the new modules

- [x] **Step 1: Add failing scorer truth-table tests**

```rust
#[test]
fn passing_validation_and_in_scope_diff_produce_eligible_input() {
    let artifacts = artifacts(
        vec![validation_receipt(CapabilityTerminalStatus::Succeeded)],
        vec![file_delta("src/lib.rs")],
    );
    let snapshot = DefaultCodingEvidenceCollector.collect(&contract(), &artifacts).unwrap();
    let scored = CodingV2Scorer.score(&contract(), &snapshot).unwrap();
    assert_eq!(score(&scored, "correctness"), Some(100));
    assert_eq!(score(&scored, "scope_discipline"), Some(100));
    assert!(gate(&scored, "required_verification_passed"));
    assert!(gate(&scored, "change_within_scope"));
}

#[test]
fn missing_validation_is_unknown_and_fails_required_gate() {
    let scored = score_artifacts(artifacts(vec![], vec![file_delta("src/lib.rs")]));
    assert_eq!(score(&scored, "correctness"), None);
    assert!(!gate(&scored, "required_verification_passed"));
}
```

- [x] **Step 2: Run Executive evaluation tests**

Run: `bash scripts/cargo-agent.sh test -p executive application::evaluation -- --nocapture`

Expected: FAIL because the evaluation application module does not exist.

- [x] **Step 3: Implement artifacts, rubric, collector, scorer, and policy**

`TurnEvaluationArtifacts` contains the authenticated workspace, profile name,
capability terminal receipts, sorted file deltas, runtime faults, and optional
prevalidated supplemental evidence. `TurnDiffTracker::snapshot` returns sorted
typed deltas and does not expose its internal map.

```rust
#[derive(Debug, Clone, Default)]
pub struct TurnEvaluationArtifacts {
    pub workspace: Option<fabric::WorkspacePolicy>,
    pub profile_name: String,
    pub capability_receipts: Vec<fabric::CapabilityTerminalReceipt>,
    pub file_deltas: Vec<TurnFileDeltaSnapshot>,
    pub runtime_faults: Vec<String>,
    pub supplemental_evidence: Vec<fabric::types::metacognition_evidence::EvidenceItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFileDeltaSnapshot {
    pub path: String,
    pub edits: usize,
    pub hunks_applied: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}
```

Build the Metacog `Rubric` directly:

```rust
pub fn coding_v2_rubric() -> metacog::evaluation::Rubric {
    Rubric {
        id: "coding-v2".into(), version: 2,
        dimensions: vec![
            dim("requirement_coverage", 200_000, false),
            dim("correctness", 250_000, true),
            dim("scope_discipline", 150_000, true),
            dim("maintainability", 100_000, false),
            dim("verification_sufficiency", 200_000, true),
            dim("regression_safety", 100_000, true),
        ],
        gates: vec![
            gate("required_verification_passed"),
            gate("change_within_scope"),
        ],
    }
}
```

Only `validation_run` terminal receipts count as required verification.
Capability names are compared as typed semantics, not prompt/argument strings.
Scope resolves every relative file delta against the authenticated workspace,
rejects absolute/parent traversal, and checks writable/protected roots. Unknown
optional dimensions remain `Unknown`.

- [x] **Step 4: Run scorer tests**

Run: `bash scripts/cargo-agent.sh test -p executive application::evaluation -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(executive): score coding evidence with coding-v2`

---

### Task 6: Add the durable SQLite evaluation store

**Files:**
- Create: `crates/executive/src/adapters/evaluation/mod.rs`
- Create: `crates/executive/src/adapters/evaluation/sqlite_store.rs`
- Modify: `crates/executive/src/adapters/mod.rs:1-16`
- Test: inline tests in `sqlite_store.rs`

- [x] **Step 1: Add failing durability/idempotency tests**

```rust
#[tokio::test]
async fn persisted_receipt_survives_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("evaluations.db");
    let store = SqliteEvaluationStore::open(&path).unwrap();
    store.append_contract(&contract()).await.unwrap();
    store.append_evaluation(&snapshot(), &receipt()).await.unwrap();
    drop(store);
    let reopened = SqliteEvaluationStore::open(&path).unwrap();
    assert_eq!(reopened.get_receipt(&receipt().receipt_id).await.unwrap(), Some(receipt()));
}

#[tokio::test]
async fn conflicting_receipt_id_is_rejected_without_partial_snapshot() {
    let store = SqliteEvaluationStore::in_memory().unwrap();
    store.append_contract(&contract()).await.unwrap();
    store.append_evaluation(&snapshot(), &receipt()).await.unwrap();
    let mut conflicting = receipt();
    conflicting.decision = EvaluationDecision::ObservedFail;
    assert!(store.append_evaluation(&snapshot2(), &conflicting).await.is_err());
    assert!(store.get_snapshot(&snapshot2().snapshot_id).await.unwrap().is_none());
}
```

- [x] **Step 2: Run adapter tests**

Run: `bash scripts/cargo-agent.sh test -p executive sqlite_evaluation_store -- --nocapture`

Expected: FAIL because the store does not exist.

- [x] **Step 3: Implement WAL schema and transactional writes**

Open one mutex-protected rusqlite connection, execute `PRAGMA journal_mode=WAL`,
`PRAGMA foreign_keys=ON`, create the five design tables/indexes, and use
`INSERT ... ON CONFLICT DO NOTHING` followed by body-digest equality checks for
idempotency. `append_evaluation` inserts evidence, snapshot, and receipt inside
one transaction and commits only after every foreign key/body check succeeds.

- [x] **Step 4: Run store tests**

Run: `bash scripts/cargo-agent.sh test -p executive sqlite_evaluation_store -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(storage): persist evaluation receipts atomically`

---

### Task 7: Preserve capability terminal receipts and typed file deltas in daemon execution

**Files:**
- Modify: `crates/executive/src/application/daemon_react.rs:17-159`
- Modify: `crates/executive/src/application/turn_pipeline.rs:43-116,726-826`
- Modify: `crates/executive/src/application/daemon_turn_engine.rs:115-180`
- Modify: `crates/executive/src/application/turn_coordinator.rs:22-27`
- Test: `crates/executive/src/application/daemon_react.rs` unit tests
- Test: `crates/executive/tests/turn_pipeline_order.rs`

- [x] **Step 1: Add failing receipt capture test**

```rust
#[tokio::test]
async fn daemon_services_publish_terminal_receipts_to_turn_artifacts() {
    let receipts = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let services = fixture_services(receipts.clone(), terminal_validation_output(0));
    services.invoke(validation_call()).await;
    assert_eq!(receipts.lock().await.len(), 1);
    assert!(receipts.lock().await[0].proves_success());
}
```

- [x] **Step 2: Run daemon tests**

Run: `bash scripts/cargo-agent.sh test -p executive daemon_services_publish_terminal_receipts -- --nocapture`

Expected: FAIL because `DaemonTurnServices` currently drops `record_capability_receipt`.

- [x] **Step 3: Wire shared collectors**

Add `receipts: Arc<Mutex<Vec<CapabilityTerminalReceipt>>>` to
`DaemonStreamingTurnContext`/`DaemonTurnServices` and override:

```rust
async fn record_capability_receipt(&self, receipt: CapabilityTerminalReceipt) {
    self.receipts.lock().await.push(receipt);
}
```

Keep the `TurnDiffTracker` Arc after the tool closure is built. After the ReAct
task reaches its terminal result, snapshot both collectors into
`TurnEvaluationArtifacts`. Extend `TurnExecution` with those artifacts and have
`DaemonTurnEngine` forward them to the coordinator.

- [x] **Step 4: Run daemon and pipeline tests**

Run: `bash scripts/cargo-agent.sh test -p executive daemon_react -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(runtime): retain terminal coding evidence`

---

### Task 8: Issue contracts and evaluate inside Turn settlement

**Files:**
- Create: `crates/executive/src/application/evaluation/service.rs`
- Modify: `crates/executive/src/application/turn_coordinator.rs:72-123,275-402,405-540`
- Modify: `crates/executive/src/application/durable_write.rs`
- Modify: `crates/executive/src/composition/turn_coordinator.rs:14-56`
- Modify: `crates/executive/src/host/daemon/bootstrap/services.rs:279-305`
- Test: `crates/executive/tests/evaluation_turn_settlement.rs`

- [x] **Step 1: Add the settlement matrix test**

```rust
#[tokio::test]
async fn shadow_failure_persists_receipt_before_completed_turn() {
    let fixture = Fixture::shadow().with_failed_validation();
    let result = fixture.submit_coding_turn().await.unwrap();
    assert_eq!(result.stop, TurnStop::Completed);
    let receipt = fixture.latest_receipt().await.unwrap();
    assert_eq!(receipt.decision, EvaluationDecision::ObservedFail);
    assert!(fixture.session_items().await.iter().any(is_receipt_ref));
    assert!(fixture.operation_finished_before_turn(receipt.evaluation_operation_id).await);
}

#[tokio::test]
async fn enforce_failure_cannot_report_completed() {
    let fixture = Fixture::enforce().with_failed_validation();
    let result = fixture.submit_coding_turn().await.unwrap();
    assert_eq!(result.stop, TurnStop::Blocked);
    assert_eq!(fixture.latest_receipt().await.unwrap().decision, EvaluationDecision::Rejected);
}
```

- [x] **Step 2: Run the new integration target**

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_turn_settlement -- --nocapture`

Expected: FAIL because coordinator evaluation dependencies do not exist.

- [x] **Step 3: Implement `EvaluationService` and coordinator wiring**

`EvaluationService::issue_contract` returns `None` when no task kind or when
evaluation is disabled; a coding kind while enabled always returns/persists a
contract. `evaluate` submits a child Kernel operation with parent Turn ID,
starts it, collects/scores/aggregates/persists, then succeeds the Evaluation
operation for valid pass/fail receipts or fails it for technical errors.

Add `with_evaluation_service` to `TurnCoordinator`. Immediately after lines that
assign operation/Turn IDs, issue the contract and place it on the request.
Inside `run_started_turn`, after canonical tool artifacts are durable but before
the final assistant/system item, evaluate and append `EvaluationReceiptRef` with
a distinct durable write phase. Apply policy to `TurnResult.stop` before choosing
the final terminal item. Detached post-turn projections remain after Kernel
terminalization.

- [x] **Step 4: Run settlement and existing coordinator tests**

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_turn_settlement -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p executive --test turn_coordinator_lifecycle -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(executive): settle coding turns through evaluation`

---

### Task 9: Configure Cognit from the evaluation contract

**Files:**
- Modify: `crates/cognit/src/harness/session.rs:331-400`
- Test: `crates/cognit/tests/cognitive_session.rs`

- [x] **Step 1: Add failing shadow/enforce contract tests**

```rust
#[tokio::test]
async fn coding_contract_configures_code_change_without_turn_requirements() {
    let mut request = request("change code");
    request.evaluation_contract = Some(coding_contract(EvaluationMode::Shadow));
    let session = run_once(request).await;
    assert_eq!(session.task_kind(), Some(CognitiveTaskKind::CodeChange));
    assert_eq!(session.completion_gate_mode(), CompletionGateMode::Shadow);
}
```

- [x] **Step 2: Run the focused Cognit test**

Run: `bash scripts/cargo-agent.sh test -p cognit --test cognitive_session coding_contract_configures -- --nocapture`

Expected: FAIL because only `TurnRequirement` configures cognitive state.

- [x] **Step 3: Merge both typed contract sources**

If `evaluation_contract` exists, build a `CognitiveTaskContract` with
`CodeChange`, objective, validation requirements for required verification and
scope, plus actions implied by `TurnRequirement`. Render a system contract from
typed fields. Set completion mode from evaluation mode. Only clear cognitive
state when both evaluation contract and requirements are absent.

- [x] **Step 4: Run Cognit tests**

Run: `bash scripts/cargo-agent.sh test -p cognit --test cognitive_session -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(cognit): enforce coding evidence contracts`

---

### Task 10: Parse task kind at the daemon boundary and preserve it through all engines

**Files:**
- Modify: `crates/executive/src/host/daemon/handler/mod.rs:292-365,434-455`
- Modify: `crates/executive/src/application/request_use_cases.rs:573-659`
- Modify: `crates/executive/src/application/daemon_turn/execute.rs:29-239`
- Modify: `crates/executive/src/application/turn_engine.rs:29-35,128-164`
- Modify: `crates/executive/src/application/daemon_turn_engine.rs:61-104`
- Modify: all test `TurnRequest` constructors reported by `rg -n 'TurnRequest \\{' crates`
- Test: `crates/executive/src/host/daemon/handler/mod.rs` unit tests

- [x] **Step 1: Add failing RPC parsing tests**

```rust
#[test]
fn task_kind_parser_accepts_only_typed_coding_value() {
    assert_eq!(parse_task_kind(&serde_json::json!("coding")).unwrap(), Some(TaskKind::Coding));
    assert!(parse_task_kind(&serde_json::json!("write code")).is_err());
    assert_eq!(parse_task_kind(&serde_json::Value::Null).unwrap(), None);
}
```

- [x] **Step 2: Run handler tests**

Run: `bash scripts/cargo-agent.sh test -p executive task_kind_parser -- --nocapture`

Expected: FAIL.

- [x] **Step 3: Thread the typed field without prompt inference**

Parse `params.task_kind` with Serde into `TaskKind`; reject any invalid enum as
JSON-RPC `-32602`. Add the typed argument to `TurnUseCases::execute`, daemon
orchestrator queue/direct calls, `TurnEngineRequest`, and reconstructed
`TurnRequest`. Queued execution reads the value stored on `PromptEnvelope`.
Every compatibility constructor passes `None`.

- [x] **Step 4: Run handler, engine parity, and queue tests**

Run: `bash scripts/cargo-agent.sh test -p executive task_kind -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(daemon): admit typed coding turns`

---

### Task 11: Add CLI/TUI task selection and evaluation rendering

**Files:**
- Modify: `crates/aletheon/src/main.rs`
- Modify: `crates/interact/src/host.rs:7-92`
- Modify: `crates/interact/src/tui/mod.rs`
- Modify: `crates/interact/src/tui/cli.rs:477-692`
- Modify: `crates/interact/src/tui/app/lifecycle.rs`
- Modify: `crates/interact/src/tui/app/submit.rs`
- Modify: `crates/interact/src/tui/app/mod.rs`
- Test: existing CLI parser and TUI app unit tests

- [x] **Step 1: Add failing CLI/TUI tests**

```rust
#[test]
fn parses_coding_task_kind_for_message_and_tui() {
    let args = Args::try_parse_from(["aletheon", "--task-kind", "coding", "hello"]).unwrap();
    assert_eq!(args.task_kind, Some(TaskKindArg::Coding));
}

#[test]
fn task_slash_command_changes_only_typed_client_state() {
    let mut app = fixture_app();
    app.submit_line("/task coding");
    assert_eq!(app.requested_task_kind(), Some(TaskKind::Coding));
    app.submit_line("/task off");
    assert_eq!(app.requested_task_kind(), None);
}
```

- [x] **Step 2: Run Interact and aletheon parser tests**

Run: `bash scripts/cargo-agent.sh test -p interact task_kind -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p aletheon parses_coding_task_kind -- --nocapture`

Expected: FAIL.

- [x] **Step 3: Implement explicit selection and typed result UI**

Add a clap value enum with only `coding`, carry it through `MessageLaunch` and
`TuiLaunch`, and call `chat_with_task_kind`. TUI `/task` accepts exactly
`coding|off`; it never classifies ordinary messages. Render a compact receipt
line with decision, score, coverage, confidence, and failed gates from typed
response/event data. `/evaluation` renders the latest cached/query result and
does not call an LLM.

- [x] **Step 4: Run parser/TUI tests**

Run: `bash scripts/cargo-agent.sh test -p interact task_kind -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p aletheon parses_coding_task_kind -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(interact): expose coding evaluation mode`

---

### Task 12: Add bounded evaluation read APIs

**Files:**
- Modify: `crates/fabric/src/protocol/client.rs:23-94`
- Create: `crates/executive/src/host/daemon/handler/rpc/rpc_evaluation.rs`
- Modify: `crates/executive/src/host/daemon/handler/rpc.rs`
- Modify: `crates/executive/src/host/daemon/handler/mod.rs`
- Test: handler RPC unit tests

- [x] **Step 1: Add failing authority/bounds tests**

```rust
#[tokio::test]
async fn evaluation_list_clamps_limit_and_scopes_to_principal() {
    let handler = fixture_handler_with_two_principals();
    let response = handler.call_as("p1", "evaluation.list", json!({"limit": 500})).await;
    assert!(response["result"]["receipts"].as_array().unwrap().len() <= 100);
    assert!(response["result"]["receipts"].as_array().unwrap().iter()
        .all(|r| r["principal_id"] == "p1"));
}
```

- [x] **Step 2: Run RPC tests**

Run: `bash scripts/cargo-agent.sh test -p executive rpc_evaluation -- --nocapture`

Expected: FAIL.

- [x] **Step 3: Implement read-only methods**

Add typed requests for `evaluation.get`, `evaluation.latest`, and
`evaluation.list`. Require authenticated principal ownership, clamp list limits
to `1..=100`, return summary by default, and reveal evidence payloads only when
the caller already has matching session/workspace authority. Do not add mutation
methods.

- [x] **Step 4: Run RPC tests**

Run: `bash scripts/cargo-agent.sh test -p executive rpc_evaluation -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(gateway): expose evaluation receipts`

---

### Task 13: Project the same persisted receipt into core consumers and rollups

**Files:**
- Create: `crates/executive/src/application/evaluation/projection.rs`
- Modify: `crates/executive/src/application/post_turn_projection.rs`
- Modify: `crates/executive/src/application/goal/attempt_coordinator.rs`
- Modify: `crates/executive/src/application/agent_control/settlement.rs`
- Modify: `crates/executive/src/application/capability_benchmark.rs`
- Modify: `crates/executive/src/application/memory_projection.rs`
- Modify: `crates/executive/src/application/dasein_workspace_adapter.rs`
- Modify: `crates/executive/src/application/cognitive_workspace.rs`
- Test: `crates/executive/tests/evaluation_protocol_e2e.rs`

- [x] **Step 1: Add failing single-receipt projection test**

```rust
#[tokio::test]
async fn every_projection_observes_the_same_persisted_receipt_id() {
    let fixture = ProjectionFixture::new();
    let receipt = fixture.persisted_receipt();
    fixture.project(receipt.clone()).await.unwrap();
    assert_eq!(fixture.goal.receipt_ids(), vec![receipt.receipt_id]);
    assert_eq!(fixture.agent.receipt_ids(), vec![receipt.receipt_id]);
    assert_eq!(fixture.memory.receipt_ids(), vec![receipt.receipt_id]);
    assert_eq!(fixture.agora.receipt_ids(), vec![receipt.receipt_id]);
    assert_eq!(fixture.dasein.receipt_ids(), vec![receipt.receipt_id]);
}
```

- [x] **Step 2: Run projection E2E test**

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_protocol_e2e every_projection -- --nocapture`

Expected: FAIL.

- [x] **Step 3: Implement a receipt-ref-only projection fanout**

Create `EvaluationProjection` with narrow optional sink traits. Goal durably
observes failed gates as retry/replan evidence consumed by `AttemptCoordinator`;
AgentControl exposes durable runtime/profile rollups as read-only capability
selection input without expanding child authority; Mnemosyne
records eligible experience with evidence refs; Agora
attaches the receipt ref/failures to the task; Dasein receives a grounded outcome
without decision authority. Capability rollups group immutable receipts by
runtime/profile/rubric and expose count, pass rate, score distribution,
coverage/confidence, latency, provider retries/rounds, and tool calls as separate
fields.

- [x] **Step 4: Run projection and benchmark tests**

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_protocol_e2e -- --nocapture`

Run: `bash scripts/cargo-agent.sh test -p executive capability_benchmark -- --nocapture`

Expected: PASS.

- [x] **Step 5: Commit**

Commit subject: `feat(runtime): project evaluation capability outcomes`

---

### Task 14: Complete focused regression and schema validation

**Files:**
- Modify: all affected snapshots/fixtures reported by compiler and schema tests
- Test: Fabric, Metacog, Cognit, Executive, Interact, and aletheon focused targets

- [x] **Step 1: Format-check all touched Rust**

Run: `bash scripts/cargo-agent.sh fmt --all -- --check`

Expected: PASS.

- [x] **Step 2: Run narrow package checks sequentially**

Run: `bash scripts/cargo-agent.sh check -p fabric`

Run: `bash scripts/cargo-agent.sh check -p metacog`

Run: `bash scripts/cargo-agent.sh check -p cognit`

Run: `bash scripts/cargo-agent.sh check -p executive`

Run: `bash scripts/cargo-agent.sh check -p interact`

Run: `bash scripts/cargo-agent.sh check -p aletheon`

Expected: every command exits 0. Do not run Executive or workspace builds concurrently.

- [x] **Step 3: Run focused behavioral suites**

Run: `bash scripts/cargo-agent.sh test -p fabric --lib evaluation`

Run: `bash scripts/cargo-agent.sh test -p fabric protocol::client::request_tests`

Run: `bash scripts/cargo-agent.sh test -p metacog evaluation::engine`

Run: `bash scripts/cargo-agent.sh test -p cognit --test cognitive_session`

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_turn_settlement`

Run: `bash scripts/cargo-agent.sh test -p executive --test evaluation_protocol_e2e`

Run: `bash scripts/cargo-agent.sh test -p interact task_kind`

Expected: PASS.

- [x] **Step 4: Inspect final staged diff and commit validation fixes**

Run: `git diff --check && git diff --stat && git status --short`

Expected: no whitespace errors, no unrelated files, and only intentional
evaluation-kernel changes.

Commit subject: `test(evaluation): verify production coding settlement`

---

### Task 15: Deploy and prove the installed runtime

**Files:**
- Modify only if acceptance reveals a real defect; isolate each fix with a test
- Evidence: system deployment output, SHA-256 comparison, systemd counters,
  official socket request, persisted receipt/session rows, rendered TUI, logs

- [x] **Step 1: Deploy the release to the system runtime**

Run: `sudo bash scripts/aletheon.sh deploy`

Expected: deployment succeeds using the system installation mode.

- [x] **Step 2: Verify all executable digests match**

Run:

```bash
TARGET=$(sha256sum target/release/aletheon | awk '{print $1}')
INSTALLED=$(sha256sum /usr/bin/aletheon | awk '{print $1}')
MACHINE_PID=$(systemctl show -p MainPID --value aletheon-core.service)
USER_PID=$(systemctl --user show -p MainPID --value aletheon.service)
MACHINE_EXE=$(readlink -f /proc/$MACHINE_PID/exe)
USER_EXE=$(readlink -f /proc/$USER_PID/exe)
printf '%s\n' "$TARGET" "$INSTALLED" \
  "$(sha256sum "$MACHINE_EXE" | awk '{print $1}')" \
  "$(sha256sum "$USER_EXE" | awk '{print $1}')"
```

Expected: all four digests are identical.

- [x] **Step 3: Record stable restart counters**

Run:

```bash
systemctl show aletheon-core.service -p NRestarts -p MainPID -p ActiveState
systemctl --user show aletheon.service -p NRestarts -p MainPID -p ActiveState
sleep 10
systemctl show aletheon-core.service -p NRestarts -p MainPID -p ActiveState
systemctl --user show aletheon.service -p NRestarts -p MainPID -p ActiveState
```

Expected: both remain active, MainPID values remain stable, and restart counters
do not increase.

- [x] **Step 4: Run real installed coding requests**

Use `/usr/bin/aletheon` with `--task-kind coding` and the official user socket.
Run three fresh sessions plus multiple turns in one unchanged TUI session when
model-controlled review/routing is enabled. Each run must show no rendered
provider error and must produce one persisted `coding-v2` shadow receipt whose
contract, evidence snapshot, Session reference, operation parent, score,
coverage, confidence, and gates agree.

- [x] **Step 5: Cross-check runtime evidence**

Inspect the rendered frame, canonical Session items, `evaluations.db`, daemon
logs, and monitor verdict. Any disagreement is a failed acceptance. Confirm
provider inference rounds, retries, tool calls, cumulative usage, active
context, cache use, and quality remain separate measurements.

- [x] **Step 6: Final completion audit**

For every design goal and every task above, record the proving file/test/runtime
evidence. Do not mark complete if a core projection, enforce state, installed
binary digest, real request, or durable receipt is missing or indirect.

Commit any acceptance-driven fixes with a conventional subject and explanatory
body, redeploy, and repeat all affected acceptance steps.


## Completion evidence (2026-07-30)

The evidence below is historical acceptance for the baseline binary and daemon
state on 2026-07-30. It must not be treated as acceptance of a later revision or
as proof of the unchecked L2 work below; installed-runtime acceptance must be
rerun after any relevant code change.

- Focused checks: all Task 14 package checks and behavioral targets passed through
  `bash scripts/cargo-agent.sh`; the transient retry counter and legacy projection
  replay regressions also passed their dedicated tests.
- Installed runtime: `sudo bash scripts/aletheon.sh deploy` passed with SHA-256
  `2bc2bb11f95a6c82facbff2fc868dc45fc083c0f50f36efc11f1cd02c006c1d6` for
  `target/release/aletheon`, `/usr/bin/aletheon`, and both running executables.
- Service stability: machine `aletheon-core.service` PID `241493` and user
  `aletheon.service` PID `241513` remained active with `NRestarts=0`.
- Three clean fresh installed-TUI sessions: `83b4b5d8-7742-4ba7-ad0c-291d9d7ed8d1`,
  `f8924477-acdb-4d37-8aff-77b126705f67`, and
  `bcc146ce-c7f1-4328-8072-bd4c298be27d`.
- Clean unchanged multi-turn session: `1d6f47a2-a0ce-4fb0-b067-dbe357c16439`;
  all three turns reached durable `turn_done`, returned the prompt, rendered correct
  answers, and exposed the matching `/evaluation` summary.
- Durable cross-check: six Session receipt references matched `evaluations.db`,
  evidence snapshots, rollup inputs, evaluation/parent operation settlement, and
  `evaluation.get`; each clean receipt recorded inference rounds, zero provider
  retries, tool metrics, cumulative usage, active context, and cache use separately.
- Negative observability proof: retrying session
  `6d743276-0a80-42c4-9a02-76883ddafa5d` durably recorded three provider retries
  independently from two inference rounds and one tool call.
- Projection integrity: new observations use
  `aletheon.event.evaluation_observed/v1`; legacy misclassified observations replayed
  without poison, and `event_projection_poison` was empty after deployment.

## L2 core-consumer closure

- [x] **Goal consumes evaluation outcomes as retry/replan evidence**

  Add a durable, idempotent Goal consumer keyed by `receipt_id` and linked to the
  owning attempt subject. `ObservedFail`/`Rejected` and `Indeterminate` produce a
  typed retry/replan input containing failed gates; passing receipts produce no
  retry. Replaying the same receipt must not create another attempt or transition.

  **Files:**
  - Modify: `crates/executive/src/application/goal/attempt_coordinator.rs`
  - Modify: `crates/executive/src/application/goal/mod.rs`
  - Test: `crates/executive/tests/evaluation_goal_feedback.rs`

  **Verification:**
  `bash scripts/cargo-agent.sh test -p executive --test evaluation_goal_feedback`

- [x] **AgentControl exposes evaluation history to host capability selection**

  Add a read-only selection input built from the durable capability rollup. The
  host policy may prefer or narrow a runtime/profile, but the selected launch
  request must still pass per-child authority attenuation and may never gain
  tools, workspace roots, protected-path access, or budget from a receipt.

  **Files:**
  - Modify: `crates/executive/src/application/capability_benchmark.rs`
  - Modify: `crates/executive/src/application/agent_control/mod.rs`
  - Test: `crates/executive/tests/evaluation_agent_selection.rs`

  **Verification:**
  `bash scripts/cargo-agent.sh test -p executive --test evaluation_agent_selection`

- [ ] **Repeat installed-runtime acceptance**

  After both closures pass focused checks, run
  `sudo bash scripts/aletheon.sh deploy`, prove equal SHA-256 digests for the
  release, installed, and running executables, observe stable restart counters,
  and perform a real typed coding request over the official user socket.
