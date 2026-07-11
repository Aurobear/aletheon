# Morphogenesis — Self-Evolution Pipeline

> New document — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

> Morphogenesis is Aletheon's self-evolution flow. The agent does not update code — it regenerates itself. The pipeline: run -> reflect -> mutate spec -> generate candidate -> evaluate -> migrate -> become.

**Crate:** `metacog`
**Code location:** `metacog/src/impl/morphogenesis/`
**Related modules:** [meta-runtime.md](meta-runtime.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| MorphogenesisPipeline | Design skeleton | `metacog/src/impl/morphogenesis/pipeline.rs` | Orchestrates full pipeline (all `todo!()`) |
| Candidate | Design skeleton | `metacog/src/impl/morphogenesis/candidate.rs` | Runtime candidate model |
| MutationIntent | Design skeleton | `metacog/src/impl/morphogenesis/mutation_intent.rs` | Mutation intent from reflection |
| GenomeLoader | Design skeleton | `metacog/src/impl/genome/loader.rs` | Genome loading and parsing |

---

## 1. Design Philosophy

From `README.md` section 8:

> Agent does not update code.
>
> Agent regenerates itself.
>
> Pipeline: run -> reflect -> mutate spec -> generate candidate -> evaluate -> migrate -> become

This is the fundamental distinction of Aletheon: rather than patching code in-place, the agent generates a complete new version of itself, tests it, and if it passes evaluation, migrates to become the new version. The old version is preserved for rollback.

---

## 2. The Pipeline

```
run
  |
  v
reflect          -- Analyze current performance, identify weaknesses
  |
  v
mutate spec      -- Generate MutationIntent to modify genome
  |
  v
generate candidate -- Build a new RuntimeCandidate from mutated genome
  |
  v
evaluate         -- Test candidate in sandbox, score against criteria
  |
  v
migrate          -- If evaluation passes, migrate to new runtime
  |
  v
become           -- The new version IS the agent now
```

### 2.1 Run

The agent operates normally, accumulating performance data, tool call outcomes, and user feedback.

### 2.2 Reflect

The agent analyzes its own performance. This uses the `SelfFieldOps` trait from `fabric` to read the current state, and `CognitOps` for reflection and critique.

Output: `MutationIntent` — a structured description of what should change.

### 2.3 Mutate Spec

Takes the `MutationIntent` and applies it to the current `Genome`, producing a modified genome specification.

Code location: `metacog/src/impl/morphogenesis/mutation_intent.rs`

### 2.4 Generate Candidate

Builds a `RuntimeCandidate` from the mutated genome. This involves the `RuntimeBuilder` from the MetaRuntime subsystem.

Code location: `metacog/src/impl/morphogenesis/candidate.rs`

### 2.5 Evaluate

Tests the candidate in a sandbox environment and evaluates the results. Uses `SandboxRunner` and `Evaluator` from MetaRuntime.

### 2.6 Migrate

If evaluation passes, the `MigrationManager` performs the actual migration — graceful shutdown, state transfer, startup of new version.

### 2.7 Become

The new version is now the running agent. The `LineageRecorder` records the full provenance chain.

---

## 3. Genome Model

From `README.md` section 9:

> Replace fixed architecture.
>
> genome/ -> topology, identity, boundary, memory, mutation, evaluator

The `Genome` is the complete specification of an agent's architecture. Rather than a fixed codebase, the genome defines:

```rust
pub struct Genome {
    pub topology: Topology,       // Subsystem graph and connections
    pub identity: Identity,       // Who the agent is
    pub boundary: Boundary,       // What the agent can and cannot do
    pub care: Care,               // What the agent cares about
    pub memory: MemoryConfig,     // Memory architecture configuration
    pub mutation: MutationConfig, // How the agent can change itself
    pub lifecycle: LifecycleConfig, // Lifecycle constraints
}
```

Code location: `fabric/src/types/genome.rs`

### 3.1 Topology

Defines the subsystem graph — which subsystems exist and how they connect. This replaces a fixed module structure with a declarative description.

### 3.2 Identity

From `README.md` section 3.1: The agent's sense of self, including name, version, and continuity markers.

### 3.3 Boundary

From `README.md` section 3.2: What the agent can and cannot do — permission boundaries, safety constraints, operational limits.

### 3.4 Care

From `README.md` section 3.3: What the agent cares about — values, priorities, goals that guide its behavior.

### 3.5 Memory

Memory architecture configuration — which memory backends are active, scope policies, compression strategies.

### 3.6 Mutation

Rules governing how the agent can mutate itself — allowed mutation types, safety constraints, evaluation criteria.

### 3.7 Lifecycle

Lifecycle constraints — terminal constraints (preserve continuity), meta goals, and emergent goals as described in `README.md` section 11.

---

## 4. Continuity Anchor

From `README.md` section 10:

> Minimal invariant.
> Preserve: lineage, memory relation, user relation, migration history.
> Not fixed personality.

The `LineageRecorder` in the MetaRuntime preserves these invariants across all morphogenesis cycles. The agent's personality may change, but its lineage, relationships, and history are maintained.

---

## 5. Design Notes

- **All components are design skeletons** — implementation comes in a future round
- **Pipeline is async** — each stage can take significant time (especially evaluate)
- **Rollback is always available** — if migration fails, `RollbackManager` restores the previous version
- **No direct production updates** — all changes go through the full pipeline

---

## Implementation Summary

**Code locations:**
- `metacog/src/impl/morphogenesis/pipeline.rs` — `MorphogenesisPipeline<M: MetaRuntimeOps>` (design skeleton)
- `metacog/src/impl/morphogenesis/candidate.rs` — Candidate model
- `metacog/src/impl/morphogenesis/mutation_intent.rs` — Mutation intent model
- `metacog/src/impl/genome/loader.rs` — `GenomeLoader`
- `fabric/src/types/genome.rs` — `Genome` struct definition

**Key types:**
- `MorphogenesisPipeline` — orchestrates the full pipeline
- `PipelineResult` — success/failure with candidate, evaluation, migration details
- `Genome` — complete agent architecture specification
- `MutationIntent` — structured description of desired changes
- `RuntimeCandidate` — a new version of the runtime to be evaluated
