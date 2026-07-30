# Metacognition Evolution Wiring — Implementation Record

**Date:** 2026-07-31
**Status:** Implemented; operator-configurable and default-off pending installed-runtime acceptance

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
- Production bootstrap now supplies explicit genome, lineage, sandbox-receipt,
  mutation-state, and rollback-store paths below the injected state root.
- `GovernedEvolutionProposer` requires two durable `coding-v2` receipts from
  distinct profiles in the same session, binds their digest and the exact
  verification hash into one pending `DaseinModification` approval, and owns no
  admission/apply authority.
- Human approval resolution uses Kernel admission for `metacog.apply`, verifies
  the approval against durable mutation status, calls the governed Metacog
  service, and settles the permit. Repeating an approved request resumes the
  idempotent apply path after a post-decision failure.
- Configuration exposes an independent `evolution_permitted` operator gate;
  both evolution switches remain false by default.

## Fail-closed boundary

`evolution_permitted` remains false by default. Insufficient or single-profile
evidence parks the verified candidate and creates no approval. No
model-controlled auto-apply was added.

## Validation

```bash
bash scripts/cargo-agent.sh test -p metacog --lib
```

Result: 112 passed, 0 failed.

Additional focused checks:

```bash
bash scripts/cargo-agent.sh test -p metacog --test service_contract
bash scripts/cargo-agent.sh test -p executive --test evolution_integration
bash scripts/cargo-agent.sh test -p executive evolution_proposer --lib
bash scripts/cargo-agent.sh test -p executive --test turn_use_case_ports
bash scripts/cargo-agent.sh test -p cognit config --lib
```
