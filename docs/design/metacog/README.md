# Metacog Crate — Meta-Cognition and Evolution

> Code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `metacog`
**Purpose:** The self-modification engine. Reads its own genome, generates candidate runtime modifications, tests them in sandbox, evaluates results, and migrates to improved versions. No direct production updates.

---

## Internal Structure

```
metacog/src/
  lib.rs                          # Crate root
  genome/                         # Genome model (loader, specifications)
  governance/                     # MetaRuntime, RollbackManager
  evolution/                      # Self-evolution pipeline (candidates, mutation)
  evaluation/                     # HIL evidence, test result evaluation
  evidence/                       # Evidence collection and verification
  experience/                     # Experience recording and replay
  improvement/                    # Improvement proposals and tracking
  problem/                        # Problem ledger and diagnostics
  reflection/                     # Reflective analysis after execution
  adapters/                       # External adapter implementations
```

The old technical-layer roots `core/`, `bridge/`, and `impl/` were removed
during the architecture decoupling refactor. Metacog now uses feature-owned
modules directly under `src/`.

## Key Concepts

- **Genome** — Complete agent architecture specification (topology, identity, boundary, care, memory, mutation, lifecycle). Defined in `crates/metacog/src/genome/`.
- **MetaRuntime** — The engine that reads, modifies, tests, and migrates. Implements `MetaRuntimeOps` trait from `metacog/src/governance/contracts.rs`.
- **Morphogenesis** — The self-evolution pipeline: run -> reflect -> mutate -> generate -> evaluate -> migrate -> become.
- **Continuity Anchor** — The minimal invariant preserved across all mutations: lineage, memory relation, user relation, migration history.

## Related Docs

- [meta/meta-runtime.md](meta-runtime.md) — MetaRuntime design (SelfReader, SpecEditor, RuntimeBuilder, SandboxRunner, Evaluator, RollbackManager, MigrationManager, LineageRecorder)
- [meta/morphogenesis.md](morphogenesis.md) — Morphogenesis pipeline and Genome model
