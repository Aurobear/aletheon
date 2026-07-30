# Metacognition Evolution Wiring — Implementation Record

**Date:** 2026-07-31
**Status:** In progress; verify prerequisites implemented, governed proposal/apply remains default-off

## Requirement anchors

- Candidate validation must evaluate the exact genome and never launch Cargo
  (`docs/plans/2026-07-30-metacognition-evolution-wiring-design.md:254-274`).
- Apply and rollback must persist and read back the current genome and rollback
  snapshots (`docs/plans/2026-07-30-metacognition-evolution-wiring-design.md:299-307,339-354`).
- Human resolution and an authority-issued `metacog.apply` permit remain required
  before apply (`docs/plans/2026-07-30-metacognition-evolution-wiring-design.md:276-297`).

## Implemented

- `SandboxRunner` is now a bounded, in-process, candidate-aware replay suite.
  It hashes the exact candidate, validates identity, numerical ranges, genome-only
  targets, mandatory approval/sandbox flags, and baseline safety boundaries. It
  optionally persists a typed `CandidateSandboxReceipt` atomically.
- `DefaultMetaRuntime` compares each candidate with the effective baseline.
- `RollbackManager` persists a digest-bound snapshot stack atomically, loads it
  after restart, and verifies snapshot integrity before rollback.
- Migration and rollback write the effective genome with temp-file, fsync,
  rename, parse/read-back, and exact serialized-value comparison before success.
- Snapshot creation moved from candidate generation to migration, so verification
  cannot pollute rollback history.

## Remaining fail-closed boundary

`evolution_permitted` remains false by default. The runtime must not be described
as production evolution until the single approval repository creates an evidence-
bound `DaseinModification` proposal and the human resolve path mints the kernel
permit and invokes apply. No model-controlled auto-apply was added.

## Validation

```bash
bash scripts/cargo-agent.sh test -p metacog --lib
```

Result: 112 passed, 0 failed.
