# General Metacognition and Governed Self-Evolution Implementation Plan

> **For agentic workers:** Implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not execute later phases until the current phase gate passes.

**Goal:** Build a domain-neutral, evidence-backed Metacog loop that records experiences and problems, scores outcomes, proposes governed improvements, and measures whether accepted evolution actually improves the system.

**Architecture:** Stable cross-crate contracts live in Fabric; Metacog is organized by feature rather than `core/bridge/impl`; append-only stores back evidence, problems, proposals, and experiments. Domain adapters publish generic evidence and cannot call mutation internals directly. Existing approval, sandbox, lineage, migration, and rollback behavior remains authoritative.

**Tech Stack:** Rust, Serde/JSON, JSONL append-only persistence, SHA-256 integrity/fingerprints, Tokio, existing Fabric clock/approval/meta-runtime contracts.

**Requirement source:** `docs/plans/2026-07-23-general-metacognition-evolution-design.md`

---

## 0. Mandatory execution rules

- Do not invoke `cargo` directly. Use `bash scripts/cargo-agent.sh <cargo arguments>`.
- Run the narrowest package/test target that proves the current change.
- Do not run concurrent workspace or `executive` builds.
- Do not touch unrelated robot/provider work already present in the worktree.
- Inspect the staged diff before every commit.
- Each phase is a separate reviewable commit with a conventional subject and explanatory body.
- Moves in Phase 1 are behavior-preserving. Do not add new metacognitive semantics during those moves.
- Coding is not implemented until the generic contracts and persistence gates pass.

## 1. Requirement-to-task coverage

| Design requirement | Plan tasks |
|---|---|
| Generic experience/evidence model (`spec:§5.1-5.2`) | Tasks 5-7 |
| Versioned scoring, confidence, hard gates (`spec:§5.3-5.4`) | Tasks 8-9 |
| Append-only problem ledger (`spec:§6`) | Tasks 10-11 |
| Reflection and proposals remain separate (`spec:§7`) | Tasks 12-14 |
| Evolution experiments and causal lineage (`spec:§8`) | Tasks 15-16 |
| Remove `core/bridge/impl` layout (`spec:§9.1-9.2`) | Tasks 1-4 |
| Event-sourced persistence (`spec:§10`) | Tasks 6, 11, 14, 16 |
| Coding as first domain adapter (`spec:§11`) | Tasks 18-20 |
| Governance invariants (`spec:§12`) | Tasks 13, 15, 17 |
| Failure behavior (`spec:§13`) | Every negative-path test; Task 17 |
| Generic-domain and evolution acceptance (`spec:§14`) | Tasks 17, 20-21 |

## Phase 1 — Behavior-preserving feature-first migration

### Task 1: Lock the current Metacog public surface

**Files:**
- Create: `crates/metacog/tests/public_surface.rs`
- Read: `crates/metacog/src/lib.rs`
- Read: `crates/metacog/src/core/types.rs`

- [ ] **Step 1: Add a compile-time public-surface test**

```rust
use metacog::{DefaultMetaRuntime, EvaluationResult, EvolutionConfig, GenomeMeta};

#[test]
fn legacy_public_surface_remains_available_during_layout_migration() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DefaultMetaRuntime>();
    let config = EvolutionConfig::default();
    assert!(!config.auto_evolve);
    let _evaluation = EvaluationResult {
        passed: true,
        reasons: Vec::new(),
    };
    let _: Option<GenomeMeta> = None;
}
```

- [ ] **Step 2: Run the baseline test**

Run:

```bash
bash scripts/cargo-agent.sh test -p metacog --test public_surface
```

Expected: PASS before any move.

- [ ] **Step 3: Record the baseline test count**

Run:

```bash
bash scripts/cargo-agent.sh test -p metacog
```

Expected: PASS; paste the exact test count into the phase handoff.

- [ ] **Step 4: Commit the protection test**

Commit subject: `test(metacog): lock public surface before module migration`

### Task 2: Move Genome into its feature module

**Files:**
- Move: `crates/metacog/src/impl/genome/loader.rs` → `crates/metacog/src/genome/loader.rs`
- Move: `GenomeMeta`, `IdentityExt`, `CareExt`, `GenomeRule`, `ReasoningConfig`, `EvolutionConfig`, `GenomeChange`, and `ChangeType` from `crates/metacog/src/core/types.rs` → `crates/metacog/src/genome/model.rs`
- Move: `crates/metacog/src/impl/meta_runtime/self_reader.rs` → `crates/metacog/src/governance/self_reader.rs`
- Move: `crates/metacog/src/impl/meta_runtime/spec_editor.rs` → `crates/metacog/src/governance/spec_editor.rs`
- Create: `crates/metacog/src/genome/mod.rs`
- Modify: `crates/metacog/src/lib.rs`
- Modify: `crates/metacog/src/bridge/genome_bridge.rs` imports only; the file move occurs in Task 4
- Test: existing Genome tests plus `crates/metacog/tests/public_surface.rs`

Note: `self_reader.rs` (runtime genome introspection) and `spec_editor.rs`
 (spec mutation) are runtime operations, not genome data. They belong to
 `governance/` — the layer that orchestrates reading and modifying the
 running system. `genome/` contains only the data model (`model.rs`) and
 file-based persistence (`loader.rs`).

- [ ] **Step 1: Move Genome-owned files without changing behavior**

Use `git mv` for `loader.rs`, `self_reader.rs`, and `spec_editor.rs`; rename only `spec_editor.rs` to `editor.rs`. Retain all function bodies unchanged and update imports from `crate::core::types` to `crate::genome`.

- [ ] **Step 2: Create the feature facade**

```rust
mod loader;
mod model;

pub use loader::GenomeLoader;
pub use model::{
    CareExt, ChangeType, EvolutionConfig, GenomeChange, GenomeMeta, GenomeRule,
    IdentityExt, ReasoningConfig,
};
```

- [ ] **Step 2b: Wire runtime operations into governance**

In `crates/metacog/src/governance/mod.rs`, add after the existing declarations:

```rust
mod self_reader;
mod spec_editor;

pub(crate) use self_reader::SelfReader;
pub(crate) use spec_editor::SpecEditor;
```

- [ ] **Step 3: Keep temporary crate-root compatibility re-exports**

In `crates/metacog/src/lib.rs`:

```rust
pub mod genome;
pub use genome::{
    CareExt, ChangeType, EvolutionConfig, GenomeChange, GenomeMeta, GenomeRule,
    IdentityExt, ReasoningConfig,
};
```

- [ ] **Step 4: Run focused tests**

```bash
bash scripts/cargo-agent.sh test -p metacog genome
bash scripts/cargo-agent.sh test -p metacog --test public_surface
```

Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `refactor(metacog): move genome into feature module`

### Task 3: Move governed evolution and service code

**Files:**
- Move: `crates/metacog/src/impl/meta_runtime/evaluator.rs` → `crates/metacog/src/evolution/candidate_evaluator.rs`
- Move: `crates/metacog/src/impl/meta_runtime/lineage.rs` → `crates/metacog/src/evolution/lineage.rs`
- Move: `crates/metacog/src/impl/meta_runtime/migration.rs` → `crates/metacog/src/evolution/migration.rs`
- Move: `crates/metacog/src/impl/meta_runtime/rollback.rs` → `crates/metacog/src/evolution/rollback.rs`
- Move: `crates/metacog/src/impl/meta_runtime/sandbox_runner.rs` → `crates/metacog/src/evolution/sandbox_runner.rs`
- Move: `crates/metacog/src/impl/morphogenesis/candidate.rs` → `crates/metacog/src/evolution/candidate.rs`
- Move: `crates/metacog/src/impl/morphogenesis/pipeline.rs` → `crates/metacog/src/evolution/pipeline.rs`
- Move: `EvaluatorSpec`, `EvaluatorMetric`, and `EvaluationResult` from `crates/metacog/src/core/types.rs` → `crates/metacog/src/evolution/model.rs`
- Move: `crates/metacog/src/service.rs` → `crates/metacog/src/governance/service.rs`
- Move: `crates/metacog/src/core/traits.rs` → `crates/metacog/src/governance/runtime.rs`
- Create: `crates/metacog/src/evolution/mod.rs`
- Create: `crates/metacog/src/governance/mod.rs`
- Modify: `crates/metacog/src/lib.rs`

- [ ] **Step 1: Move files with `git mv` only**

Do not rename public types or change algorithms in this step.

- [ ] **Step 2: Define narrow facades**

`evolution/mod.rs`:

```rust
mod candidate;
mod candidate_evaluator;
mod lineage;
mod migration;
mod model;
mod pipeline;
mod rollback;
mod sandbox_runner;

pub(crate) use candidate::CandidateGenerator;
pub(crate) use candidate_evaluator::Evaluator;
pub(crate) use lineage::LineageTracker;
pub(crate) use migration::MigrationManager;
pub use model::{EvaluationResult, EvaluatorMetric, EvaluatorSpec};
pub use pipeline::{MorphogenesisPipeline, PipelineResult};
pub use rollback::RollbackManager;
pub(crate) use sandbox_runner::SandboxRunner;
```

`governance/mod.rs`:

```rust
mod runtime;
mod service;

pub use runtime::DefaultMetaRuntime;
pub use service::{DefaultMetacogService, MetacogError};
```

- [ ] **Step 3: Preserve root exports**

```rust
pub mod evolution;
pub mod governance;
pub use evolution::{EvaluationResult, EvaluatorMetric, EvaluatorSpec};
pub use governance::{DefaultMetaRuntime, DefaultMetacogService, MetacogError};
```

- [ ] **Step 4: Run service and contract tests**

```bash
bash scripts/cargo-agent.sh test -p metacog --test service_contract
bash scripts/cargo-agent.sh test -p metacog --test public_surface
```

Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `refactor(metacog): group evolution and governance features`

### Task 4: Remove `core`, `bridge`, and `impl`

**Files:**
- Move: `crates/metacog/src/outcome_verifier.rs` → `crates/metacog/src/evaluation/outcome.rs`
- Move: `crates/metacog/src/hil_evidence_verifier.rs` → `crates/metacog/src/evaluation/hil_evidence.rs`
- Move: `crates/metacog/src/core/meta_cognition.rs` → `crates/metacog/src/adapters/dasein.rs`

**Transition note — mood-based fallback:** The current `MetaCognition::decide()`
at `crates/metacog/src/core/meta_cognition.rs:66-105` uses `Stimmung`
(Angst/Langeweile/Neugier) to trigger evolution every 20 turns. The daemon
calls this after each turn via Dasein. Moving it to `adapters/dasein.rs`
preserves this behavior as the active evolution driver until the new
`ReflectionEngine` (Phase 4) and `ProposalPromoter` (Task 14) are fully
wired. The mood-based adapter becomes a fallback: once evidence-backed
reflection produces proposals, those take priority. Until then, the daemon's
evolution loop continues uninterrupted.
- Move: `crates/metacog/src/bridge/candidate_bridge.rs` → `crates/metacog/src/evolution/candidate_bridge.rs`
- Move: `crates/metacog/src/bridge/genome_bridge.rs` → `crates/metacog/src/genome/bridge.rs`
- Move: `crates/metacog/src/impl/morphogenesis/mutation_intent.rs` → `crates/metacog/src/improvement/promotion.rs`
- Move: `MorphogenesisCandidate`, `GenomePatch`, and `PatchOperation` from `crates/metacog/src/core/types.rs` → `crates/metacog/src/improvement/model.rs`
- Remove empty: `crates/metacog/src/core/`, `bridge/`, `impl/`
- Modify: `crates/metacog/src/lib.rs`

- [ ] **Step 1: Add destination module facades before moves**

```rust
// adapters/mod.rs
mod dasein;
pub use dasein::*;

// evaluation/mod.rs
pub mod hil_evidence;
pub mod outcome;

// improvement/mod.rs
mod model;
pub mod promotion;
pub use model::{GenomePatch, MorphogenesisCandidate, PatchOperation};
```

- [ ] **Step 2: Move and update imports**

Add these exact declarations:

```rust
// evolution/mod.rs
mod candidate_bridge;
pub use candidate_bridge::CandidateBridge;

// genome/mod.rs
mod bridge;
pub use bridge::GenomeBridge;

// lib.rs
pub mod adapters;
pub mod evaluation;
pub mod improvement;
pub use evolution::CandidateBridge;
pub use genome::GenomeBridge;
```

Delete the now-empty legacy `mod.rs` files. Do not leave
`#[path = "impl/mod.rs"]` in `lib.rs`.

- [ ] **Step 3: Prove forbidden directories are gone**

```bash
test ! -e crates/metacog/src/core
test ! -e crates/metacog/src/bridge
test ! -e crates/metacog/src/impl
! rg -n 'r#impl|path = "impl/mod.rs"' crates/metacog/src
```

Expected: all commands exit 0.

- [ ] **Step 4: Run the full crate suite**

```bash
bash scripts/cargo-agent.sh test -p metacog
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `refactor(metacog): retire technical-layer module layout`

## Phase 2 — Generic Fabric contracts and ingestion

### Task 5: Add generic experience and evidence contracts

**Files:**
- Create: `crates/fabric/src/types/metacognition_experience.rs`
- Create: `crates/fabric/src/types/metacognition_evidence.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Create: `crates/fabric/tests/metacognition_contract.rs`

Note: The contracts are split into two files to avoid a single monolith.
 `metacognition_experience.rs` owns `DomainId`, `ExperienceId`,
 `SubjectId`, `ExperienceEnvelope`, and `ExperienceOutcome`.
 `metacognition_evidence.rs` owns `EvidenceId`, `EvidenceKind`,
 `EvidenceTrust`, and `EvidenceItem`. Scoring and report types are added
 later in Task 8 and may warrant their own file depending on total size.

- [ ] **Step 1: Write failing serde contract tests**

Tests must construct and round-trip:

```rust
let experience = ExperienceEnvelope {
    schema_version: 1,
    experience_id: ExperienceId("exp-1".into()),
    domain: DomainId::new("synthetic").unwrap(),
    subject: SubjectId("component-a".into()),
    goal_ref: Some("goal-1".into()),
    started_at_ms: 100,
    completed_at_ms: Some(200),
    outcome: ExperienceOutcome::Succeeded,
    correlations: BTreeMap::from([("task".into(), "task-1".into())]),
    evidence: vec![EvidenceId("ev-1".into())],
};
```

Also verify `DomainId::new("")` and control characters are rejected.

- [ ] **Step 2: Run and observe compile failure**

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
```

Expected: FAIL because the module/types do not exist.

- [ ] **Step 3: Implement the value objects**

`metacognition_experience.rs` must define:

```rust
pub const METACOGNITION_SCHEMA_V1: u16 = 1;

pub struct ExperienceId(pub String);
pub struct SubjectId(pub String);
pub struct DomainId(String);

pub enum ExperienceOutcome { Succeeded, Failed, Cancelled, TimedOut, Unknown }

pub struct ExperienceEnvelope {
    pub schema_version: u16,
    pub experience_id: ExperienceId,
    pub domain: DomainId,
    pub subject: SubjectId,
    pub goal_ref: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub outcome: ExperienceOutcome,
    pub correlations: BTreeMap<String, String>,
    pub evidence: Vec<EvidenceId>,
}
```

`metacognition_evidence.rs` must define:

```rust
pub struct EvidenceId(pub String);

pub enum EvidenceKind {
    Assertion, Observation, ActionResult, VerificationResult, Metric,
    Artifact, HumanFeedback, PolicyDecision, RuntimeFault,
}

pub enum EvidenceTrust { Authoritative, Corroborated, Unverified }

pub struct EvidenceItem {
    pub schema_version: u16,
    pub evidence_id: EvidenceId,
    pub experience_id: ExperienceId,
    pub kind: EvidenceKind,
    pub source: String,
    pub producer: String,
    pub captured_at_ms: i64,
    pub payload: serde_json::Value,
    pub sha256: String,
    pub trust: EvidenceTrust,
    pub freshness_ms: Option<u64>,
    pub redacted: bool,
}
```

Derive `Debug`, `Clone`, `PartialEq`, `Eq` where payload permits, and Serde traits. Validate IDs at constructors; do not expose unchecked construction for `DomainId`.

- [ ] **Step 4: Pass tests**

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
```

Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `feat(fabric): add generic metacognition contracts`

### Task 6: Implement evidence integrity and append-only store

**Files:**
- Create: `crates/metacog/src/evidence/model.rs`
- Create: `crates/metacog/src/evidence/integrity.rs`
- Create: `crates/metacog/src/evidence/store.rs`
- Modify: `crates/metacog/src/evidence/mod.rs`
- Create: `crates/metacog/tests/evidence_store.rs`

- [ ] **Step 1: Write failing store tests**

Cover:

```rust
#[tokio::test]
async fn duplicate_id_with_same_digest_is_idempotent() { /* append twice, len == 1 */ }

#[tokio::test]
async fn duplicate_id_with_different_payload_is_rejected() { /* typed conflict */ }

#[tokio::test]
async fn reopening_jsonl_rebuilds_index() { /* append, drop, reopen, get */ }
```

- [ ] **Step 2: Run failing test**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evidence_store
```

Expected: FAIL because store types do not exist.

- [ ] **Step 3: Implement the port and JSONL adapter**

```rust
#[async_trait::async_trait]
pub trait EvidenceStore: Send + Sync {
    async fn append(&self, item: EvidenceItem) -> Result<AppendOutcome, EvidenceStoreError>;
    async fn get(&self, id: &EvidenceId) -> Result<Option<EvidenceItem>, EvidenceStoreError>;
    async fn list_for_experience(
        &self,
        id: &ExperienceId,
    ) -> Result<Vec<EvidenceItem>, EvidenceStoreError>;
}

pub enum AppendOutcome { Appended, AlreadyPresent }
```

Calculate SHA-256 over canonical serialized payload bytes. Reject a supplied digest that does not match. Serialize one versioned event per JSONL line; flush before acknowledging append.

- [ ] **Step 4: Pass tests**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evidence_store
```

Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `feat(metacog): persist integrity-checked evidence`

### Task 7: Implement experience ingestion

**Files:**
- Create: `crates/metacog/src/experience/model.rs`
- Create: `crates/metacog/src/experience/ingest.rs`
- Create: `crates/metacog/src/experience/mod.rs`
- Create: `crates/metacog/tests/experience_ingest.rs`

- [ ] **Step 1: Write tests for malformed, duplicate, and missing evidence**

A valid experience with unknown evidence IDs must be rejected with `MissingEvidence`; an exact duplicate must return `AlreadyPresent`.

- [ ] **Step 2: Run failing test**

```bash
bash scripts/cargo-agent.sh test -p metacog --test experience_ingest
```

- [ ] **Step 3: Implement `ExperienceIngestor`**

```rust
pub struct ExperienceIngestor<E, S> {
    evidence: E,
    store: S,
}

impl<E: EvidenceStore, S: ExperienceStore> ExperienceIngestor<E, S> {
    pub async fn ingest(
        &self,
        envelope: ExperienceEnvelope,
    ) -> Result<AppendOutcome, ExperienceIngestError>;
}
```

Validate schema, completion time ordering, all evidence references, and idempotency before append.

- [ ] **Step 4: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test experience_ingest
```

Commit subject: `feat(metacog): ingest validated experiences`

## Phase 3 — Evidence-backed scoring and problems

### Task 8: Add rubric and evaluation report contracts

**Files:**
- Create: `crates/fabric/src/types/metacognition_evaluation.rs`
- Extend: `crates/fabric/src/types/mod.rs`
- Extend: `crates/fabric/src/lib.rs`
- Extend: `crates/fabric/tests/metacognition_contract.rs`

- [ ] **Step 1: Add failing tests for unknown dimensions and hard gates**

Construct a report where one dimension is `Unknown`; verify it is absent from the weighted denominator. Construct score 95 plus a failed safety gate; verify `eligible == false`.

- [ ] **Step 2: Implement types**

```rust
pub struct RubricId(pub String);
pub enum DimensionValue { Scored(u8), Unknown }
pub struct DimensionScore {
    pub name: String,
    pub value: DimensionValue,
    pub weight_millis: u32,
    pub evidence: Vec<EvidenceId>,
    pub reasons: Vec<String>,
}
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub evidence: Vec<EvidenceId>,
}
pub struct EvaluationReport {
    pub rubric: RubricId,
    pub rubric_version: u32,
    pub dimensions: Vec<DimensionScore>,
    pub gates: Vec<GateResult>,
    pub weighted_total_millis: Option<u32>,
    pub evidence_coverage_millis: u16,
    pub confidence_millis: u16,
    pub eligible: bool,
}
```

Use integer fixed-point fields for persisted scores; do not persist floating-point totals.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
```

Commit subject: `feat(fabric): define versioned evaluation reports`

### Task 9: Implement deterministic evaluation engine

**Files:**
- Create: `crates/metacog/src/evaluation/model.rs`
- Create: `crates/metacog/src/evaluation/rubric.rs`
- Create: `crates/metacog/src/evaluation/engine.rs`
- Modify: `crates/metacog/src/evaluation/mod.rs`
- Create: `crates/metacog/tests/evaluation_engine.rs`

- [ ] **Step 1: Write table-driven score tests**

Cases: all applicable, one unknown, zero applicable, failed hard gate, missing evidence, and weight overflow rejection.

- [ ] **Step 2: Run failing test**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evaluation_engine
```

- [ ] **Step 3: Implement evaluator port and fixed-point calculation**

```rust
#[async_trait::async_trait]
pub trait Evaluator: Send + Sync {
    async fn evaluate(
        &self,
        experience: &ExperienceEnvelope,
        evidence: &[EvidenceItem],
        rubric: &Rubric,
    ) -> Result<EvaluationReport, EvaluationError>;
}
```

Use checked integer arithmetic. `eligible` is true only when all mandatory gates pass and a weighted total exists.

- [ ] **Step 4: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evaluation_engine
```

Commit subject: `feat(metacog): score evidence with versioned rubrics`

### Task 10: Define problem records and lifecycle

**Files:**
- Create: `crates/metacog/src/problem/model.rs`
- Create: `crates/metacog/src/problem/fingerprint.rs`
- Create: `crates/metacog/src/problem/mod.rs`
- Create: `crates/metacog/tests/problem_model.rs`

- [ ] **Step 1: Write lifecycle and fingerprint tests**

Verify identical normalized signatures produce the same SHA-256 fingerprint; rubric-version changes produce different fingerprints. Reject `Resolved -> Active`; allow `Resolved -> Regressed`.

- [ ] **Step 2: Implement model**

```rust
pub enum ProblemState {
    Observed, Confirmed, Active, Mitigated, Resolved,
    Disputed, AcceptedRisk, Regressed,
}
pub enum ProblemSeverity { Info, Low, Medium, High, Critical }
pub struct ProblemRecord { /* fields required by spec:§6 */ }
pub struct ProblemTransition { /* event id, old/new, reason, evidence, time */ }
```

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test problem_model
```

Commit subject: `feat(metacog): model durable problem lifecycles`

### Task 11: Implement append-only problem ledger

**Files:**
- Create: `crates/metacog/src/problem/ledger.rs`
- Create: `crates/metacog/src/problem/projection.rs`
- Create: `crates/metacog/tests/problem_ledger.rs`

- [ ] **Step 1: Write append/rebuild/regression tests**

Test idempotent events, invalid transitions, occurrence count, restart projection rebuild, and preservation of historical evidence.

- [ ] **Step 2: Implement event log and projection**

```rust
#[async_trait::async_trait]
pub trait ProblemLedger: Send + Sync {
    async fn observe(&self, finding: ProblemFinding) -> Result<ProblemId, ProblemError>;
    async fn transition(&self, event: ProblemTransition) -> Result<(), ProblemError>;
    async fn get(&self, id: &ProblemId) -> Result<Option<ProblemRecord>, ProblemError>;
    async fn active(&self) -> Result<Vec<ProblemRecord>, ProblemError>;
}
```

Never rewrite old JSONL lines. Rebuild current records solely by replaying events.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test problem_ledger
```

Commit subject: `feat(metacog): add append-only problem ledger`

## Phase 4 — Reflection, proposals, and governed promotion

### Task 12: Add reflection reports

**Files:**
- Create: `crates/metacog/src/reflection/model.rs`
- Create: `crates/metacog/src/reflection/engine.rs`
- Modify: `crates/metacog/src/reflection/mod.rs`
- Create: `crates/metacog/tests/reflection_engine.rs`

- [ ] **Step 1: Write a synthetic-domain reflection test**

Feed three confirmed problems with a shared category and verify one recurring pattern, explicit contrary evidence, and an observation recommendation. Verify no `MutationIntent` appears in the API.

- [ ] **Step 2: Implement deterministic aggregation first**

```rust
pub trait ReflectionEngine: Send + Sync {
    fn reflect(&self, input: ReflectionInput) -> Result<ReflectionReport, ReflectionError>;
}
```

The first engine groups by deterministic fingerprint/category. Model-generated narrative is out of scope for this task.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test reflection_engine
```

Commit subject: `feat(metacog): reflect on recurring problem evidence`

### Task 13: Define improvement proposals and governance states

**Files:**
- Create: `crates/metacog/src/improvement/model.rs`
- Create: `crates/metacog/src/improvement/registry.rs`
- Modify: `crates/metacog/src/improvement/mod.rs`
- Create: `crates/metacog/tests/improvement_registry.rs`

- [ ] **Step 1: Write tests proving proposals cannot self-approve**

Test lifecycle `Proposed -> PendingApproval -> Accepted/Rejected/Expired`. Reject a Metacog principal attempting to approve its own privileged proposal.

- [ ] **Step 2: Implement registry port**

```rust
pub enum ProposalState { Proposed, PendingApproval, Accepted, Rejected, Expired, Promoted }
pub struct ImprovementProposal { /* exact spec:§7 fields */ }

#[async_trait::async_trait]
pub trait ImprovementRegistry: Send + Sync {
    async fn propose(&self, proposal: ImprovementProposal) -> Result<(), ProposalError>;
    async fn decide(&self, decision: ProposalDecision) -> Result<(), ProposalError>;
    async fn accepted(&self, id: &ProposalId) -> Result<ImprovementProposal, ProposalError>;
}
```

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test improvement_registry
```

Commit subject: `feat(metacog): govern improvement proposals`

### Task 14: Persist proposals and promote accepted proposals

**Files:**
- Create: `crates/metacog/src/improvement/store.rs`
- Modify: `crates/metacog/src/improvement/promotion.rs`
- Create: `crates/metacog/tests/improvement_promotion.rs`

- [ ] **Step 1: Write negative promotion tests**

Reject unapproved, expired, irreversible-without-rollback, and evidence-free proposals. Accept only an approved proposal with target problem IDs and a validation/rollback plan.

- [ ] **Step 2: Implement the narrow promotion bridge**

```rust
pub trait ProposalPromoter: Send + Sync {
    fn promote(
        &self,
        proposal: &ImprovementProposal,
    ) -> Result<fabric::MutationIntent, PromotionError>;
}
```

The bridge constructs an intent; it does not migrate or approve it.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test improvement_promotion
```

Commit subject: `feat(metacog): promote approved improvements to intents`

## Phase 5 — Evolution experiments and post-deployment learning

### Task 15: Add evolution experiment model and thresholds

**Files:**
- Create: `crates/metacog/src/evolution/experiment.rs`
- Modify: `crates/metacog/src/evolution/mod.rs`
- Create: `crates/metacog/tests/evolution_experiment.rs`

- [ ] **Step 1: Write decision tests**

Cover promote, retain, rollback, reject, and inconclusive. A safety-gate regression must force rollback regardless of average score improvement.

- [ ] **Step 2: Implement fixed-point comparison**

```rust
pub enum ExperimentDecision { Promote, Retain, Rollback, Reject, Inconclusive }
pub struct EvolutionExperiment { /* baseline, candidate, targets, thresholds, window */ }
pub struct ExperimentOutcome { /* reports, regressions, decision */ }
```

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evolution_experiment
```

Commit subject: `feat(metacog): evaluate evolution experiments`

### Task 16: Persist causal lineage from problem to measured outcome

**Files:**
- Modify: `crates/metacog/src/evolution/lineage.rs`
- Create: `crates/metacog/src/evolution/experiment_store.rs`
- Create: `crates/metacog/tests/evolution_lineage.rs`

- [ ] **Step 1: Write a full causal-chain test**

Persist and reload:

```text
problem -> proposal -> mutation intent -> candidate -> approval
        -> sandbox evaluation -> migration -> experiment outcome
```

Verify every link is addressable after restart.

- [ ] **Step 2: Extend lineage with typed references**

Do not embed entire evidence payloads in lineage; store stable IDs and hashes.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p metacog --test evolution_lineage
```

Commit subject: `feat(metacog): link evolution to measured outcomes`

### Task 17: Add generic end-to-end governance acceptance

**Files:**
- Create: `crates/metacog/tests/governed_learning_flow.rs`
- Modify only if required: `crates/metacog/src/governance/service.rs`

- [ ] **Step 1: Build the synthetic-domain flow**

The test must:

1. append authoritative evidence;
2. ingest an experience;
3. evaluate it;
4. record and confirm a problem;
5. reflect and propose an improvement;
6. prove promotion fails before approval;
7. approve through the existing authority boundary;
8. generate and sandbox a candidate;
9. record a degraded candidate result;
10. choose rollback;
11. rebuild all stores after restart.

- [ ] **Step 2: Run the test**

```bash
bash scripts/cargo-agent.sh test -p metacog --test governed_learning_flow
```

Expected: PASS with no Coding- or Robot-specific imports.

- [ ] **Step 3: Run crate and Fabric gates**

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
bash scripts/cargo-agent.sh test -p metacog
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 4: Commit**

Commit subject: `test(metacog): verify governed learning end to end`

## Phase 6 — Coding as the first domain adapter

### Task 18: Define Coding evidence adapter outside the Metacog core

**Files:**
- Create: `crates/executive/src/application/coding_metacog_adapter.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Create: `crates/executive/tests/coding_metacog_adapter.rs`

- [ ] **Step 1: Lock the existing Coding input types**

Use `fabric::CodingJobReport` and `fabric::VerificationReport` from
`crates/fabric/src/types/coding_job.rs:192-238` plus
`executive::application::coding_runtime::CodingAttemptRequest` from
`crates/executive/src/application/coding_runtime.rs:1-10`. Do not create a
second Coding job/result model.

- [ ] **Step 2: Write adapter mapping tests**

Map requirement references, repository instructions, files read, net diff, command results, test results, review findings, installed-runtime evidence, and user corrections into generic `ExperienceEnvelope`/`EvidenceItem` values.

- [ ] **Step 3: Implement only the adapter**

```rust
pub trait CodingEvidenceAdapter {
    fn capture(
        &self,
        report: &fabric::CodingJobReport,
        verification: &fabric::VerificationReport,
    ) -> Result<CapturedExperience, CodingEvidenceError>;
}
```

Metacog must not import `CodingJobReport`, `CodingAttemptRequest`, or any other Coding-private type.

- [ ] **Step 4: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_adapter
```

Commit subject: `feat(coding): publish generic metacog evidence`

### Task 19: Add the first Coding rubric and problem taxonomy

**Files:**
- Create: `crates/executive/src/application/coding_metacog_rubric.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Create: `crates/executive/tests/coding_metacog_rubric.rs`

- [ ] **Step 1: Write deterministic fixture tests**

Fixtures must cover requirement coverage, correctness, scope discipline, maintainability evidence, verification sufficiency, and regression risk. A completion message without tool/test evidence must produce low coverage, not a fabricated high score.

- [ ] **Step 2: Implement a versioned rubric builder**

```rust
pub const CODING_RUBRIC_V1: u32 = 1;
pub fn coding_rubric_v1() -> metacog::evaluation::Rubric;
```

Only the adapter/rubric module may contain Coding-specific dimension names.

- [ ] **Step 3: Pass tests and commit**

```bash
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_rubric
```

Commit subject: `feat(coding): add evidence-backed quality rubric`

### Task 20: Run a controlled Coding benchmark through Metacog

**Files:**
- Create: `crates/executive/tests/coding_metacog_e2e.rs`
- Reuse: `tests/coding/fixtures/rust_bugfix/`
- Reuse: `tests/coding/tasks/rust_bugfix.toml`
- Reuse baseline receipt schema from: `tests/coding/receipts/rust_bugfix.json`

- [ ] **Step 1: Add the acceptance flow**

The test must execute or replay a controlled Coding task, persist evidence, calculate a report, create one confirmed problem, create an unapproved proposal, and prove no mutation occurs.

- [ ] **Step 2: Add candidate comparison**

Replay baseline and candidate outcomes with the same rubric version; record `Promote` only when thresholds and all hard gates pass.

- [ ] **Step 3: Run focused tests**

```bash
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_e2e
```

Expected: PASS.

- [ ] **Step 4: Commit**

Commit subject: `test(coding): measure improvements through metacog`

## Phase 7 — Documentation, migration audit, and final gates

### Task 21: Update architecture and operator documentation

**Files:**
- Rewrite: `crates/metacog/README.md`
- Modify: `docs/arch/README.md`
- Create: `docs/deployment/metacog-problem-ledger.md`

- [ ] **Step 1: Document the final feature-first tree**

Remove all references to `src/core`, `src/bridge`, and `src/impl`.

- [ ] **Step 2: Document persistence and recovery**

Include schema version, JSONL ownership, corruption behavior, backup, replay, quarantine, retention, and redaction.

- [ ] **Step 3: Add architecture drift checks**

Extend the existing architecture check to fail if these paths return:

```text
crates/metacog/src/core
crates/metacog/src/bridge
crates/metacog/src/impl
```

- [ ] **Step 4: Run documentation/drift gates**

```bash
bash scripts/aletheon.sh acceptance architecture
bash scripts/cargo-agent.sh fmt --all -- --check
```


- [ ] **Step 5: Commit**

Commit subject: `docs(metacog): document governed learning operations`

### Task 22: Final integration audit

**Files:** No production edits unless a failing gate identifies a scoped defect.

- [ ] **Step 1: Verify forbidden coupling**

```bash
! rg -n "CodingTaskOutcome|Robot|ROS|cargo::|git2" crates/metacog/src
! find crates/metacog/src -maxdepth 1 -type d \( -name core -o -name bridge -o -name impl \) | grep .
```

Expected: exit 0. Generic uses of the word `runtime` are allowed; domain-private types are not.

- [ ] **Step 2: Verify design coverage**

For every row in §1, paste the passing test name and commit SHA into the implementation handoff.

- [ ] **Step 3: Run final owned checks serially**

```bash
bash scripts/cargo-agent.sh test -p fabric --test metacognition_contract
bash scripts/cargo-agent.sh test -p metacog
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_adapter
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_rubric
bash scripts/cargo-agent.sh test -p executive --test coding_metacog_e2e
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: all PASS. Only the integration owner may add broader workspace checks.

- [ ] **Step 4: Produce the final handoff**

The handoff must list:

```text
STATUS
SUMMARY
CHANGED_FILES
TEST_EVIDENCE
SCHEMA_MIGRATIONS
COMPATIBILITY_WINDOW
KNOWN_LIMITATIONS
ROLLBACK_PROCEDURE
```

- [ ] **Step 5: Commit any test-only gate updates**

Commit subject: `test(metacog): close governed evolution acceptance`

## 2. Completion definition

This plan is complete only when:

- the old technical-layer directories are absent;
- a synthetic domain integrates without private domain types;
- evidence, problems, proposals, and experiments survive restart;
- scores are fixed-point, versioned, confidence-qualified, and evidence-linked;
- unknown evidence is not converted to failure;
- high scores cannot override hard safety gates;
- Metacog cannot approve its own privileged proposal;
- sandbox, rollback, and lineage remain mandatory;
- candidate improvement is measured against a baseline;
- Coding works as an external domain adapter without changing generic Metacog contracts;
- all required validation commands and exact results are recorded.
