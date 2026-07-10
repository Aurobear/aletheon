# MetaRuntime — Self-Modification Engine

> New document — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

> The MetaRuntime is Aletheon's self-modification engine. It reads its own genome, generates candidate runtime modifications, tests them in sandbox, evaluates results, and migrates to improved versions. No direct production updates — all changes go through the evaluate-then-migrate pipeline.

**Crate:** `metacog`
**Code location:** `metacog/src/impl/meta_runtime/`
**Related modules:** [morphogenesis.md](morphogenesis.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| DefaultMetaRuntime | Design skeleton | `metacog/src/core/traits.rs` | Implements MetaRuntimeOps trait (all methods todo!) |
| SelfReader | Design skeleton | `metacog/src/impl/meta_runtime/self_reader.rs` | Reads current runtime state |
| SpecEditor | Design skeleton | `metacog/src/impl/meta_runtime/spec_editor.rs` | Edits genome specifications |
| RuntimeBuilder | Design skeleton | `metacog/src/impl/meta_runtime/runtime_builder.rs` | Builds candidate runtimes from specs |
| SandboxRunner | Design skeleton | `metacog/src/impl/meta_runtime/sandbox_runner.rs` | Tests candidates in sandbox |
| Evaluator | Design skeleton | `metacog/src/impl/meta_runtime/evaluator.rs` | Evaluates test results |
| RollbackManager | Design skeleton | `metacog/src/impl/meta_runtime/rollback.rs` | Rollback to previous version |
| MigrationManager | Design skeleton | `metacog/src/impl/meta_runtime/migration.rs` | Migrate to new runtime |
| LineageRecorder | Design skeleton | `metacog/src/impl/meta_runtime/lineage.rs` | Records evolution lineage |

---

## 1. Design Philosophy

From `Aletheon.md` section 7:

> MetaRuntime is a self-modification engine.
>
> Capabilities: read self, generate patch, build runtime, sandbox, rollback, migration.
>
> No direct production update.

The MetaRuntime does not modify the running system directly. Instead, it follows a pipeline: read current state -> generate candidate -> test in sandbox -> evaluate -> migrate. This ensures that every self-modification is validated before being applied.

---

## 2. MetaRuntimeOps Trait

The ABI trait that defines the MetaRuntime contract:

```rust
#[async_trait]
pub trait MetaRuntimeOps: Subsystem {
    /// Read the current genome.
    async fn read_genome(&self) -> Result<Genome>;

    /// Generate a candidate runtime from a mutation intent.
    async fn generate_candidate(&self, intent: &MutationIntent) -> Result<RuntimeCandidate>;

    /// Test a candidate in sandbox.
    async fn sandbox_test(&self, candidate: &RuntimeCandidate) -> Result<TestResult>;

    /// Evaluate a candidate after testing.
    async fn evaluate(&self, candidate: &RuntimeCandidate, test: &TestResult) -> Result<Evaluation>;

    /// Migrate to a new runtime.
    async fn migrate(&self, candidate: &RuntimeCandidate) -> Result<MigrationResult>;

    /// Rollback to the previous runtime version.
    async fn rollback(&self) -> Result<()>;

    /// Get the current runtime version.
    fn current_version(&self) -> Version;
}
```

Code location: `base/src/meta.rs` (trait), `metacog/src/core/traits.rs` (implementation)

---

## 3. Component Descriptions

### 3.1 SelfReader

Reads the current runtime state — the genome, active configuration, running subsystems, and their health status. This is the "read self" capability that provides the baseline for mutation decisions.

Code location: `metacog/src/impl/meta_runtime/self_reader.rs`

### 3.2 SpecEditor

Edits genome specifications based on mutation intents. Takes a `MutationIntent` (from the morphogenesis pipeline's reflection phase) and produces modified genome sections.

Code location: `metacog/src/impl/meta_runtime/spec_editor.rs`

### 3.3 RuntimeBuilder

Builds candidate runtime artifacts from edited genome specifications. This involves code generation, configuration synthesis, and dependency resolution.

Code location: `metacog/src/impl/meta_runtime/runtime_builder.rs`

### 3.4 SandboxRunner

Tests candidate runtimes in an isolated sandbox environment. Runs the candidate's test suite, checks for regressions, and produces `TestResult` with pass/fail details.

Code location: `metacog/src/impl/meta_runtime/sandbox_runner.rs`

### 3.5 Evaluator

Evaluates test results against quality criteria. Produces an `Evaluation` with a score, recommendation (accept/reject/needs-review), and rationale.

Code location: `metacog/src/impl/meta_runtime/evaluator.rs`

### 3.6 RollbackManager

Manages rollback to previous runtime versions. Stores version snapshots and can restore the system to any prior version if a migration fails or is rejected.

Code location: `metacog/src/impl/meta_runtime/rollback.rs`

### 3.7 MigrationManager

Handles the actual migration to a new runtime version. This includes graceful shutdown of the current runtime, state transfer, and startup of the new runtime.

Code location: `metacog/src/impl/meta_runtime/migration.rs`

### 3.8 LineageRecorder

Records the evolution lineage — every mutation, evaluation, and migration is recorded with full provenance. This preserves the continuity anchor (lineage, memory relation, user relation, migration history) as described in `Aletheon.md` section 10.

Code location: `metacog/src/impl/meta_runtime/lineage.rs`

---

## 4. Key Types (from base)

```rust
pub struct RuntimeCandidate {
    pub id: String,
    pub version: Version,
    pub genome: Genome,
    pub artifacts: Vec<PathBuf>,
}

pub struct TestResult {
    pub passed: bool,
    pub total: usize,
    pub failures: usize,
    pub details: Vec<String>,
}

pub struct Evaluation {
    pub score: f64,
    pub recommendation: Recommendation, // Accept / Reject / NeedsReview
    pub rationale: String,
}

pub struct MigrationResult {
    pub success: bool,
    pub from_version: Version,
    pub to_version: Version,
    pub message: String,
}
```

Code location: `base/src/meta.rs`

---

## Implementation Summary

**Code locations:**
- `metacog/src/core/traits.rs` — `DefaultMetaRuntime` (design skeleton, all methods `todo!()`)
- `metacog/src/impl/meta_runtime/self_reader.rs` — SelfReader
- `metacog/src/impl/meta_runtime/spec_editor.rs` — SpecEditor
- `metacog/src/impl/meta_runtime/runtime_builder.rs` — RuntimeBuilder
- `metacog/src/impl/meta_runtime/sandbox_runner.rs` — SandboxRunner
- `metacog/src/impl/meta_runtime/evaluator.rs` — Evaluator
- `metacog/src/impl/meta_runtime/rollback.rs` — RollbackManager
- `metacog/src/impl/meta_runtime/migration.rs` — MigrationManager
- `metacog/src/impl/meta_runtime/lineage.rs` — LineageRecorder

**All components are design skeletons.** Implementation comes in a future round.
