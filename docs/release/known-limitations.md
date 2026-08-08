# Known Limitations

This document tracks the honest known limitations of the Aletheon runtime.
It is published alongside each release. The authoritative acceptance evidence
for any tagged release is the machine-generated scoreboard bound to the
immutable release candidate, not this document.

## Not Yet Accepted

These capabilities have implementation code and focused tests but have not
passed a release-level acceptance gate. They remain operational blockers.

### Physical HIL (H1)

- **Status**: Not performed.
- **Scope**: Physical hardware-in-the-loop validation with a real robot.
- **Impact**: A tagged release may claim only the exact MuJoCo simulation
  evidence its gate consumed. Physical robot support requires separate
  operator-supplied H1 evidence.
- **ADR Reference**: Physical HIL/real robot is subject to an independent
  safety gate per ADR.

### Robot R8 Live Post-Stage Tasks

- **Status**: Operator-driven manual gate after the stage-rc job.
- **Validation entrypoint**: `scripts/aletheon.sh acceptance robot-r8`
- **Scope**: Fresh positive (completed) and negative (failed + safe-stop)
  Robot task evidence produced on the exact staged binary, with daemon
  restart and SQLite cross-validation.
- **Impact**: The `r8-evidence-handoff.yml` workflow pauses for operator
  approval after staging the RC. Live task execution and report capture are
  operator responsibilities. The automated verifier validates the resulting
  receipts but does not execute the tasks. The first Release gate fails closed
  until that artifact exists; after handoff completes, the operator re-runs
  the failed Release jobs with the same run ID.

### Release Promotion and Tag/Publish/Smoke

- **Status**: Release-time operational gates, not yet executed for any
  tagged release.
- **Scope**: `dev` to `main` promotion, immutable RC acceptance, semantic
  tag creation, GitHub Release publishing, and post-release smoke.
- **Impact**: These gates are defined in the release workflow but their
  execution for a specific tag remains a release-time operation. No static
  documentation can substitute for having run them.

### Environment Protection/Configuration

- **Status**: Required for the `robot-r8-release` GitHub Environment.
- **Scope**: The self-hosted runner's topology, credentials, database paths,
  and environment variables must be pre-configured and protected.
- **Impact**: Without proper environment configuration, the handoff workflow
  cannot produce valid evidence. This is an operational prerequisite, not a
  code-level gate.

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
