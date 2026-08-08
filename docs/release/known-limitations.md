# Known Limitations

This document tracks the honest known limitations of the Aletheon runtime.
It is published alongside each release. The authoritative acceptance evidence
for any tagged release is the machine-generated scoreboard bound to the
immutable release candidate, not this document.

## Not Yet Accepted

These capabilities have implementation code and focused tests but have not
passed a release-level acceptance gate.

### Physical HIL (H1)

- **Status**: Not performed.
- **Scope**: Physical hardware-in-the-loop validation with a real robot.
- **Impact**: A tagged release may claim only the exact MuJoCo simulation
  evidence its gate consumed. Physical robot support requires separate
  operator-supplied H1 evidence.
- **ADR Reference**: Physical HIL/real robot is subject to an independent
  safety gate per ADR.

### Robot R8 Acceptance

- **Status**: Mandatory gate consumed by the release workflow.
- **Entrypoint**: `scripts/aletheon.sh acceptance robot-r8`
- **Scope**: Fresh installed acceptance with positive (completed) and negative
  (failed + safe-stop) Robot task evidence, daemon restart, and SQLite
  cross-validation.
- **Impact**: The automated release workflow downloads R8 evidence produced
  by the self-hosted `r8-evidence-handoff.yml` workflow for the exact
  GITHUB_SHA and verifies it with
  `tests/coding/harness/r8_evidence_verifier.py`. A release will not publish
  until authoritative positive and negative R8 receipts are consumed. Fixture
  tests (`scripts/tests/test_robot_r8_evidence.py`) prove checker behavior
  only and are not substitutes for fresh installed Robot acceptance.

### Convergence Changes Versus Tagged Releases

- **Status**: Implemented on the convergence branch; pending the
  mandatory release gate (installed scoreboard, Robot simulation
  acceptance, promotion, tag, and post-release smoke). Source tests
  alone do not constitute release validation.
- **Scope**: R3 provides the versioned typed command envelope, U1 uses one
  reducer for durable projection and live overlays, and S1 uses indexed
  per-session append with the required scale benchmark matrix.
- **Impact**: These capabilities become a release claim only when the exact
  immutable RC passes the installed scoreboard, Robot simulation gate,
  promotion, tag, and post-release smoke workflow. Branch-level source tests
  are not substitutes for that evidence.

## Experimental (feature-gated)

These capabilities exist behind feature flags, environment variables, or as
`examples/` only. They are not part of any release claim.

- **ContainerHost**: Docker/Podman container lifecycle management.
- **io_uring IPC backend**: High-performance IPC using Linux io_uring.
- **Local/Offline Model**: Configured Ollama through the OpenAI-compatible
  path; llama.cpp routing remains planned.
- **Self-evolution loop**: Agent-driven code/config modification example.
- **eBPF kernel awareness**: Partial implementation; not production.

## Design-Only

These capabilities exist as design artifacts only. No production code.

- Android / Embedded targets
- Cross-platform (macOS / Windows)

## Migration and Rollback

Per `config/release/migration-matrix.toml`:

- **Forward-only**: Data-changing transitions (`kind = "migration"`) are
  forward-only. Binary-only rollback after a data change is forbidden.
- **Matching-data-and-binary rollback**: Components with `data_change = false`
  support rollback when the binary and data versions match.
- **Backup required**: All transitions require backup before migration.
- **Mixed-version operation**: Forbidden. All components must operate at the
  same version.

## Release Acceptance

A tagged release is bound to the machine-generated acceptance scoreboard for
the exact immutable release candidate. The authoritative gate is recorded in
the acceptance run directory (`manifest.json`, `scoreboard.json`). No static
claims in this document override that evidence.
