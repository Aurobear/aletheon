# Meta Crate — Self-Modification Engine

> Code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

**Crate:** `metacog`
**Purpose:** The self-modification engine. Reads its own genome, generates candidate runtime modifications, tests them in sandbox, evaluates results, and migrates to improved versions. No direct production updates.

---

## Internal Structure

```
metacog/src/
  lib.rs                          # Crate root
  core/                           # Core trait implementations
    traits.rs                     # DefaultMetaRuntime (design skeleton)
    types.rs                      # Re-exported types
  bridge/                         # Bridge to other subsystems
    mod.rs
  impl/                           # Concrete implementations
    mod.rs
    genome/                       # Genome model
      mod.rs
      loader.rs                   # GenomeLoader — loads genome from files
    meta_runtime/                 # MetaRuntime components
      mod.rs
      self_reader.rs              # SelfReader — reads current runtime state
      spec_editor.rs              # SpecEditor — edits genome specifications
      runtime_builder.rs          # RuntimeBuilder — builds candidate runtimes
      sandbox_runner.rs           # SandboxRunner — tests in sandbox
      evaluator.rs                # Evaluator — evaluates test results
      rollback.rs                 # RollbackManager — rollback to previous version
      migration.rs                # MigrationManager — migrate to new runtime
      lineage.rs                  # LineageRecorder — records evolution lineage
    morphogenesis/                # Self-evolution pipeline
      mod.rs
      pipeline.rs                 # MorphogenesisPipeline — orchestrates full flow
      candidate.rs                # RuntimeCandidate model
      mutation_intent.rs          # MutationIntent from reflection
```

## Key Concepts

- **Genome** — Complete agent architecture specification (topology, identity, boundary, care, memory, mutation, lifecycle). Defined in `base/src/genome.rs`.
- **MetaRuntime** — The engine that reads, modifies, tests, and migrates. Implements `MetaRuntimeOps` trait from `base/src/meta.rs`.
- **Morphogenesis** — The self-evolution pipeline: run -> reflect -> mutate -> generate -> evaluate -> migrate -> become.
- **Continuity Anchor** — The minimal invariant preserved across all mutations: lineage, memory relation, user relation, migration history.

## Related Docs

- [meta/meta-runtime.md](meta-runtime.md) — MetaRuntime design (SelfReader, SpecEditor, RuntimeBuilder, SandboxRunner, Evaluator, RollbackManager, MigrationManager, LineageRecorder)
- [meta/morphogenesis.md](morphogenesis.md) — Morphogenesis pipeline and Genome model
