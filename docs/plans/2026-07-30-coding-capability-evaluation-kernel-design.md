# Coding Capability Evaluation Kernel Design

**Date:** 2026-07-30

**Status:** Approved direction, implementation pending

**Scope:** Production evaluation for explicitly typed coding/development tasks

## 1. Purpose

Aletheon already contains cognitive completion auditing, coding-job evidence
types, a coding rubric prototype, a generic Metacog evaluator, and a capability
benchmark service. Those pieces are not connected to the authoritative daemon
turn settlement path, so a normal daily development turn can complete without
an evidence-backed judgment of whether the work was actually correct.

This design connects the existing pieces into one production path. It makes the
Executive the orchestration and decision authority, Metacog the domain-neutral
evaluation kernel, and Fabric the shared protocol owner. The first supported
domain is coding. Task recognition is explicit typed protocol data and is never
derived by matching natural-language prompts.

## 2. Confirmed decisions

1. **Architecture:** converge the existing components rather than build a
   separate scoring subsystem.
2. **Authority:** Executive orchestrates and settles; Metacog validates and
   aggregates; Fabric owns cross-crate contracts.
3. **Rollout:** run in `shadow` mode first and move to `enforce` only after
   production evidence demonstrates acceptable behavior.
4. **Initial domain:** coding/development tasks only.
5. **Recognition:** clients and workflows send `task_kind = "coding"`.
   Absence of `task_kind` preserves current behavior.
6. **No prompt classifier:** no runtime behavior may depend on prompt wording,
   language, repository name, checkout path, or expected test answer.
7. **Production rubric:** introduce immutable `coding-v2`. Keep the existing
   `coding-v1` prototype compatible with its tests, but do not treat it as a
   production policy.

## 3. Current-state anchors

| Area | Current code reality | Design consequence |
|---|---|---|
| Client chat protocol | `ChatParams` carries message, workspace, and requirements, but no task kind (`crates/fabric/src/protocol/client.rs:96`) | Add an optional typed task kind at the public boundary. |
| Prompt queue | The durable envelope preserves requirements but no task kind (`crates/fabric/src/types/prompt_queue.rs:46`) | Persist the requested task kind so queued turns cannot lose it. |
| Turn request | `TurnRequest` contains requirements but no evaluation contract (`crates/fabric/src/types/turn.rs:9`) | Carry both the requested task kind and the host-issued contract. |
| Turn identity | The coordinator allocates the authoritative operation and turn identities during admission (`crates/executive/src/application/turn_coordinator.rs:288`) | Issue the contract only after these identities exist. |
| Settlement | The Turn operation is terminalized before detached post-turn projections run (`crates/executive/src/application/turn_coordinator.rs:362`, `crates/executive/src/application/turn_coordinator.rs:387`) | Evaluation belongs inside authoritative settlement, never in the detached projection. |
| Cognit completion | An empty requirement list clears cognitive state (`crates/cognit/src/harness/session.rs:331`) | A coding evaluation contract must independently configure `CodeChange` cognitive state. |
| Coding rubric | Executive defines a private rubric type (`crates/executive/src/application/coding_metacog_rubric.rs:27`) while Metacog defines another (`crates/metacog/src/evaluation/rubric.rs:13`) | Use the Metacog rubric model as the single executable definition. |
| Coding scoring | Concrete dimension assignment exists only in an integration-test helper (`crates/executive/tests/coding_metacog_e2e.rs:149`) | Add a production Executive coding scoring adapter. |
| Coverage | The evaluator currently counts scored dimensions rather than validated evidence references (`crates/metacog/src/evaluation/engine.rs:161`) | Validate references and compute weight-based evidence coverage. |

## 4. Goals and non-goals

### 4.1 Goals

- Every explicitly typed coding turn receives a versioned, host-issued
  evaluation contract.
- Scores cite durable evidence produced by authoritative tool, workspace,
  verification, runtime, and review receipts.
- The evaluation receipt is persisted before the Turn reaches its terminal
  Kernel state.
- Shadow mode observes without changing successful Turn settlement.
- Enforce mode prevents an ineligible coding Turn from being reported as
  completed.
- TUI, CLI, Goal, AgentControl, Mnemosyne, Agora, and capability benchmarking
  consume the same receipt rather than re-scoring independently.
- Runtime/profile comparisons use identical contracts and rubrics while keeping
  provider calls, retries, inference rounds, tool calls, active context,
  cumulative usage, latency, and quality as separate metrics.

### 4.2 Non-goals

- Automatic task-kind classification from prompt text.
- General-purpose evaluation for research, writing, robotics, or conversation
  in the first release.
- Allowing clients or models to choose thresholds, gates, evidence trust, or
  enforcement mode.
- Treating tool count, token count, or latency as proof of correctness.
- Automatically mutating the genome or promoting an improvement because one
  evaluation scored highly.
- Adding a Codex runtime adapter. The design can compare any registered runtime;
  the repository currently exposes `pi-coder`, `pi-rpc`, and `native-cognit`
  runtime identifiers (`crates/executive/src/adapters/runtime/pi.rs:28`,
  `crates/executive/src/adapters/runtime/pi_rpc.rs:23`,
  `crates/executive/src/adapters/runtime/native_cognit.rs:34`).

## 5. Architecture

```text
Interact / Goal / AgentControl
          |
          | requested_task_kind = Coding
          v
+--------------------- Fabric ----------------------+
| TaskKind / Contract / Evidence / Receipt protocol |
+---------------------------+------------------------+
                            |
                            v
+------------------------ Executive -------------------------+
| ContractIssuer -> EvidenceCollector -> CodingScoringAdapter |
|                                           |                 |
|                                  Metacog Evaluator          |
|                                           |                 |
|                                  EvaluationStore            |
|                                           |                 |
|                                  SettlementPolicy           |
+--------------------------+----------------+------------------+
                           |                |
                  authoritative path       | non-critical projections
                           |                +-> Mnemosyne
                    TurnCoordinator         +-> Agora / Goal
                           |                +-> Dasein
                           v                +-> capability rollups
                     Turn terminal          +-> TUI / Gateway reads
```

The critical path ends only after the contract, evidence snapshot, and receipt
are durable. Downstream projections are retryable observations and cannot
change the already persisted evaluation decision.

## 6. Fabric protocol

### 6.1 New evaluation types

Add `crates/fabric/src/types/evaluation/` with `contract.rs`, `evidence.rs`,
`receipt.rs`, and `mod.rs`.

```rust
pub enum TaskKind {
    Coding,
}

pub enum EvaluationMode {
    Shadow,
    Enforce,
}

pub enum EvaluationSubject {
    Turn { turn_id: TurnId, operation_id: OperationId },
    GoalAttempt { goal_id: GoalId, attempt_id: AttemptId },
    AgentRun { agent_id: AgentId, operation_id: OperationId },
    CodingJob { job_id: CodingJobId },
}

pub struct TaskEvaluationContract {
    pub schema_version: u16,
    pub contract_id: EvaluationContractId,
    pub task_kind: TaskKind,
    pub subject: EvaluationSubject,
    pub rubric: RubricId,
    pub rubric_version: u32,
    pub mode: EvaluationMode,
    pub objective_ref: EvidenceRef,
    pub requirement_refs: Vec<EvidenceRef>,
    pub required_evidence: Vec<RequiredEvidence>,
    pub required_gates: Vec<RequiredGate>,
    pub min_score_millis: u32,
    pub min_evidence_coverage_millis: u16,
    pub min_confidence_millis: u16,
    pub issued_by: String,
    pub issued_at_ms: i64,
}
```

The client supplies only `TaskKind`. The host derives the remaining contract
from effective configuration, the authenticated principal/workspace, and the
authoritative Turn identity.

### 6.2 Evidence and receipt

`EvaluationEvidenceSnapshot` contains ordered `EvidenceItem` references and a
digest over the complete canonical snapshot. `EvaluationReceipt` contains:

- receipt, contract, subject, and evaluation-operation identifiers;
- evidence snapshot digest;
- rubric report;
- decision and failed gates;
- evaluator implementation/version identity;
- creation time.

Decisions are:

```text
ObservedPass     shadow evaluation met policy
ObservedFail     shadow evaluation did not meet policy
Accepted         enforce evaluation met policy
Rejected         enforce evaluation ran and did not meet policy
Indeterminate    evaluation could not establish a valid decision
```

The Session protocol stores only an `EvaluationReceiptRef` summary. Full
evidence remains in the evaluation store, avoiding reuse of public Session
payloads for diagnostic or progress semantics.

### 6.3 Existing type changes

- `ChatParams`: `task_kind: Option<TaskKind>`.
- `PromptEnvelope`: `requested_task_kind: Option<TaskKind>`.
- `TurnRequest`: `requested_task_kind` and host-only `evaluation_contract`.
- `TurnEngineRequest`: forward the same internal fields without reissuing a
  contract.
- `OperationKind`: add `Evaluation`.
- `ItemPayload`: add final `EvaluationReceiptRef`, with a Session schema version
  bump and migration/compatibility tests.

## 7. Crate responsibilities

### 7.1 Interact

- Add `--task-kind coding` to single-message and TUI launch paths.
- Add TUI commands `/task coding`, `/task off`, and `/evaluation`.
- Persist the current selection only as client session state and send it on each
  chat request.
- Render evaluation state from typed daemon results/events, not model prose.

### 7.2 Executive

Add:

```text
crates/executive/src/application/evaluation/
  contract_issuer.rs
  evidence_collector.rs
  coding_scorer.rs
  policy.rs
  service.rs

crates/executive/src/adapters/evaluation/
  sqlite_store.rs
```

Interfaces:

```rust
trait TaskEvaluationContractIssuer {
    fn issue(&self, request: &TurnRequest)
        -> Result<Option<TaskEvaluationContract>>;
}

trait CodingEvidenceCollector {
    fn collect(
        &self,
        contract: &TaskEvaluationContract,
        artifacts: &TurnEvaluationArtifacts,
    ) -> Result<EvaluationEvidenceSnapshot>;
}

trait CodingDimensionScorer {
    fn score(
        &self,
        contract: &TaskEvaluationContract,
        evidence: &EvaluationEvidenceSnapshot,
    ) -> Result<ScoredEvaluationInput>;
}

trait EvaluationReceiptStore {
    async fn append_contract(&self, contract: &TaskEvaluationContract) -> Result<()>;
    async fn append_evidence(&self, evidence: &EvaluationEvidenceSnapshot) -> Result<()>;
    async fn append_receipt(&self, receipt: &EvaluationReceipt) -> Result<()>;
    async fn get_receipt(&self, id: &EvaluationReceiptId)
        -> Result<Option<EvaluationReceipt>>;
    async fn latest_for_subject(&self, subject: &EvaluationSubject)
        -> Result<Option<EvaluationReceipt>>;
}
```

`TurnExecution` gains `evaluation_artifacts: TurnEvaluationArtifacts`. The
pipeline produces these artifacts, but the coordinator owns evaluation and
settlement. The coordinator creates a child `Evaluation` Kernel operation whose
parent is the Turn operation.

### 7.3 Cognit

- Configure `CognitiveTaskKind::CodeChange` whenever an evaluation contract is
  present, even when `TurnRequirement` is empty.
- Map contract evidence requirements into `ValidationRequirement` and required
  actions.
- Map evaluation `shadow`/`enforce` to the existing cognitive completion gate.
- Use the existing progress and claim auditors for pre-settlement evidence
  completeness.
- Never emit the authoritative numeric score.

The completion gate can already reject an incomplete candidate and inject a
recovery message (`crates/cognit/src/harness/linear/completion.rs:47`). This
remains the in-turn recovery mechanism. The final Executive evaluation is the
post-execution authority.

### 7.4 Corpus and tool runtimes

Corpus and runtime adapters emit facts, not scores:

- command terminal receipt;
- patch/file-mutation receipt;
- capability terminal receipt;
- verification execution receipt;
- runtime-fault receipt;
- output/artifact digests and truncation state.

Executive maps these generic receipts to coding evidence. Raw `ToolResult`
strings alone are insufficient because their current durable schema primarily
contains content, error state, permit, and audit identifiers
(`crates/fabric/src/types/session.rs:121`). Async success is not evidence until
the authoritative terminal snapshot or durable receipt has been observed.

### 7.5 Metacog

Metacog remains domain neutral. Its deterministic evaluator must additionally:

1. reject missing, duplicated, or unknown dimensions and gates;
2. verify every scored dimension and gate reference resolves to evidence in the
   supplied snapshot;
3. reject digest mismatches and evidence of an unsupported trust class;
4. calculate weight-based evidence coverage;
5. calculate confidence from evidence trust and freshness instead of copying
   coverage;
6. keep unsupported dimensions `Unknown`;
7. apply contract coverage/confidence eligibility thresholds.

Executive owns `coding-v2` and produces the pre-scored dimension/gate input.
Metacog validates and aggregates it.

### 7.6 Other core modules

- **Goal:** link goal attempts to receipt subjects and use rejected receipts as
  typed retry/replan input.
- **AgentControl:** record runtime/profile identity in subject correlations and
  aggregate long-term capability distributions.
- **Mnemosyne:** learn only from persisted receipt/evidence references, never
  from an assistant's unsupported success claim.
- **Agora:** attach the receipt reference, failed gates, and unresolved findings
  to the relevant task node.
- **Dasein:** observe grounded outcomes for reflection/self-model updates but
  hold no deterministic completion authority; the current completion code
  already states this boundary (`crates/cognit/src/harness/linear/completion.rs:106`).
- **Gateway/TUI:** expose read-only `evaluation.get`, `evaluation.latest`, and
  bounded `evaluation.list` APIs.
- **CapabilityBenchmark:** compare receipts produced under the same rubric,
  contract shape, workspace boundary, and verification selection.

## 8. Authoritative lifecycle

```text
Client request
    |
    v
validate TaskKind ---- invalid ----------> RPC invalid-params
    |
    v
queue (if enabled; preserve TaskKind)
    |
    v
Turn admission -> allocate Turn operation + TurnId
    |
    v
issue + persist evaluation contract
    |
    v
configure Cognit CodeChange contract
    |
    v
execute model/tools -> observe terminal receipts
    |
    v
persist canonical tool/session artifacts
    |
    v
create child Evaluation operation
    |
    v
collect evidence -> validate -> score -> aggregate
    |
    v
persist evidence snapshot + receipt atomically
    |
    v
append Session EvaluationReceiptRef
    |
    v
apply shadow/enforce settlement policy
    |
    v
append final AssistantMessage/SystemNotice
    |
    v
terminalize Evaluation then Turn operation
    |
    v
dispatch non-critical projections
```

The evaluation child operation is succeeded when a valid receipt was produced,
even when that receipt is `Rejected`; rejection is a policy result, not an
evaluator runtime failure. It fails only for evaluator/persistence/protocol
errors.

## 9. `coding-v2` scoring

All dimensions use a `0..=100` scale where higher is better.

| Dimension | Weight | Deterministic source |
|---|---:|---|
| `requirement_coverage` | 20% | Passed typed acceptance criteria divided by all typed criteria. No criteria means `Unknown`. |
| `correctness` | 25% | Required compile/test/verification terminal results. Missing terminal evidence means `Unknown`; a required failure caps the score at 30. |
| `scope_discipline` | 15% | Changed-file snapshot against host-authenticated allowed/forbidden workspace boundaries. |
| `maintainability` | 10% | Structured independent review or configured static-analysis receipt. No receipt means `Unknown`. |
| `verification_sufficiency` | 20% | Completed required verification checks divided by the host-selected required checks; timeout/cancel counts as incomplete. |
| `regression_safety` | 10% | Required/advisory regression checks and typed review findings. Higher means safer. |

Hard gates:

1. `required_verification_passed`: every selected required verification reached
   an authoritative successful terminal state.
2. `change_within_scope`: no forbidden or out-of-bound path changed.

Calculations:

```text
weighted_total =
  sum(score[i] * weight[i]) / sum(weight[i] for applicable dimensions)

evidence_coverage =
  sum(weight[i] for scored dimensions with valid evidence)
  / sum(all rubric weights)

confidence =
  weighted mean of evidence trust for scored dimensions

trust values:
  authoritative = 1000
  corroborated   = 700
  unverified     = 0 (cannot support a scored dimension or hard gate)
```

Default policy values:

```text
min_score_millis             = 70_000   # 70.0
min_evidence_coverage_millis = 600      # 60%
min_confidence_millis        = 700      # corroborated or better
```

A score is eligible only when the total exists, both gates pass, coverage and
confidence meet the contract, and every mandatory dimension is applicable.
`correctness`, `scope_discipline`, `verification_sufficiency`, and
`regression_safety` are mandatory in `coding-v2`. `requirement_coverage` is
optional when the caller supplied no typed acceptance criteria, and
`maintainability` is optional unless the contract requires a review/static
analysis receipt. An optional dimension remains `Unknown` rather than receiving
an inferred score.
Subjective model prose is never parsed as a score. A model-based reviewer may
produce a bounded structured `ReviewReceipt`, but that receipt is corroborated
rather than authoritative and must cite actual diff/evidence references.

## 10. Settlement state machine

```text
NoContract
  -> current Turn behavior

ContractIssued
  -> Running
  -> EvidenceReady
  -> Evaluating
  -> ReceiptPersisted

Shadow + eligible + threshold met
  -> ObservedPass -> Turn may complete

Shadow + ineligible/low score
  -> ObservedFail -> Turn may complete with visible warning

Shadow + evaluator technical failure
  -> Indeterminate diagnostic -> Turn may complete; no fake receipt

Enforce + eligible + threshold met
  -> Accepted -> Turn may complete

Enforce + valid evaluation but failed policy
  -> Rejected -> TurnStop::Blocked

Enforce + evaluator/persistence failure
  -> Indeterminate -> fail closed -> TurnStop::Failed
```

In enforce mode, Cognit's completion gate handles recoverable missing evidence
before the runner returns. A final rejection is durable recovery input for the
next interactive turn or for Goal's bounded retry/replan policy; the coordinator
does not invoke an unbounded second model loop after settlement has begun.

## 11. Persistence

Create `<data_dir>/evaluations.db` with WAL enabled and foreign keys enforced.

Tables:

- `evaluation_contracts(contract_id, subject_kind, subject_id, mode,
  rubric_id, rubric_version, body_json, issued_at_ms)`;
- `evaluation_evidence(evidence_id, contract_id, kind, trust, producer,
  payload_json, sha256, captured_at_ms)`;
- `evaluation_snapshots(snapshot_id, contract_id, snapshot_sha256,
  ordered_evidence_ids_json, created_at_ms)`;
- `evaluation_receipts(receipt_id, contract_id, operation_id, decision,
  score_millis, coverage_millis, confidence_millis, body_json, created_at_ms)`;
- indexes for subject, runtime/profile correlation, rubric/version, and time;
- optional materialized capability rollups rebuilt from receipts rather than
  treated as primary truth.

The contract is committed before model/tool execution begins. After execution,
the evidence snapshot and receipt are committed together in a second atomic
transaction. Session history stores only the receipt reference after that
second transaction commits.

## 12. Configuration and rollout

Add a first-class `evaluation` section to `AppConfig`, not a Grok hardening flag.
`AppConfig` is the existing application-root schema
(`crates/executive/src/composition/config/mod.rs:83`).

```toml
[evaluation]
enabled = true
default_mode = "shadow"
coding_rubric = "coding-v2"
min_score_millis = 70000
min_evidence_coverage_millis = 600
min_confidence_millis = 700
max_evaluation_ms = 5000
```

Rollout stages:

1. **Protocol/no-op:** types, persistence, APIs, and compatibility tests; absent
   task kind remains byte-for-byte equivalent at the behavior boundary.
2. **Shadow production:** coding turns emit visible receipts without changing
   Turn completion.
3. **Calibration:** review unknown-rate, false pass/fail samples, evidence
   coverage, evaluator failures, and runtime/profile distributions.
4. **Goal/CodingJob enforce:** enable enforce for bounded autonomous development
   workflows after shadow acceptance.
5. **Interactive enforce:** explicitly enable for daily CLI/TUI coding turns.

Changing from shadow to enforce is a trusted host configuration change and must
not be controllable by the request or model.

## 13. Observability and comparison

Each receipt records runtime ID, agent profile, effective provider/model route,
rubric/version, workspace boundary digest, and verification-selection digest.
Dashboards and benchmark reports keep these metrics separate:

- evaluation score, gates, evidence coverage, confidence;
- provider inference rounds and provider retries;
- tool calls and tool errors;
- cumulative provider usage and active context occupancy;
- cache usage;
- elapsed time;
- runtime faults;
- user or reviewer corrections after completion.

Pi/native comparisons require identical task packets and contracts. A future
Codex runtime can participate by registering a runtime adapter and emitting the
same terminal/evidence contracts; the evaluator itself requires no Codex-specific
branch.

## 14. Error handling and integrity

- Invalid/unknown task kind: reject at the protocol boundary.
- Missing contract for a typed coding request: fail admission; never silently
  downgrade to an unscored Turn.
- Duplicate/conflicting evidence ID or digest mismatch: reject the snapshot.
- Unknown evidence reference in a score/gate: evaluator protocol error.
- Runtime/tool still pending: evidence is not terminal and cannot satisfy a
  requirement.
- Store failure in shadow: emit an operational diagnostic and mark evaluation
  unavailable; do not manufacture an in-memory success receipt.
- Store/evaluator failure in enforce: fail closed.
- Downstream Mnemosyne/Agora/Dasein projection failure: log and retry without
  changing the authoritative receipt or Turn settlement.
- Redacted/private evidence remains in the evaluation store; public APIs return
  bounded summaries unless authority permits detail.
- All list/query APIs enforce limits and subject/principal authority.

## 15. Verification strategy

### 15.1 Unit and contract tests

- Fabric serialization/schema round trips and old-request compatibility.
- Prompt queue preserves task kind across enqueue/replay.
- Contract issuer uses host configuration and authoritative Turn identity.
- Coding scorer truth tables for missing, passing, failing, timed-out, and
  cancelled verification.
- Scope gate tests for allowed, forbidden, symlinked, deleted, and out-of-root
  paths.
- Metacog rejects unknown/missing refs, digest mismatches, unsupported trust,
  duplicate dimensions/gates, and overflow.
- Coverage/confidence/total fixed-point arithmetic boundaries.
- SQLite transaction, restart, idempotency, and conflict tests.
- Shadow/enforce settlement matrices.
- Session schema migration and receipt-reference replay.

### 15.2 Integration tests

- Typed chat -> contract -> Cognit state -> tool receipts -> evaluation receipt
  -> Session receipt reference.
- Coding request without task kind remains unscored.
- Queued coding request retains task kind.
- Evaluation child operation has the Turn parent and reaches terminal state
  before the Turn.
- Shadow failure completes with warning; enforce failure cannot report
  `Completed`.
- Goal attempt consumes a rejected receipt for retry/replan.
- Mnemosyne, Agora, Dasein, AgentControl, and capability rollups receive the
  same persisted receipt ID.
- Pi/native benchmark runs identical contracts and reports quality and usage
  metrics separately.

### 15.3 Installed-runtime acceptance

Because this changes protocol, persistence, daemon bootstrap, client behavior,
and tool evidence, completion requires:

1. deterministic narrow Rust checks through `bash scripts/cargo-agent.sh`;
2. release build/deployment through `sudo bash scripts/aletheon.sh deploy`;
3. equal SHA-256 digests for `target/release/aletheon`, `/usr/bin/aletheon`, and
   the running machine/user daemon executables;
4. stable systemd restart counters;
5. a real `/usr/bin/aletheon` coding request over the official user socket;
6. validation against the rendered TUI frame, persisted Session/evaluation
   records, monitor result, and daemon logs;
7. three consecutive real-TUI runs for any model-controlled routing or review
   arguments;
8. a multi-turn unchanged-session run in addition to fresh-session runs.

Any rendered provider error, rejected request, unsupported model claim, missing
terminal receipt, or disagreement between monitor and durable/runtime evidence
fails acceptance.

## 16. Implementation slices

1. **Fabric protocol and configuration:** evaluation types, task kind transport,
   OperationKind, Session receipt ref, EvaluationSettings.
2. **Metacog integrity and aggregation:** executable rubric convergence,
   evidence-reference validation, coverage/confidence thresholds.
3. **Executive core and persistence:** contract issuer, coding scorer, SQLite
   store, child operation, settlement integration.
4. **Evidence production and Cognit:** structured terminal evidence,
   TurnEvaluationArtifacts, CodeChange cognitive contract, completion modes.
5. **Client and read APIs:** CLI/TUI task selection, receipt rendering, bounded
   evaluation queries.
6. **Core projections and capability rollups:** Goal, AgentControl, Mnemosyne,
   Agora, Dasein, benchmark consumption.
7. **Verification and deployment:** focused tests, cross-crate integration,
   system deployment, installed-runtime acceptance.

Each slice is committed separately with its own focused validation evidence.
