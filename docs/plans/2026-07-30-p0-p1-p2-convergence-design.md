# P0/P1/P2 Convergence Design

**Date:** 2026-07-30  
**Status:** Approved for planning  
**Scope:** Repair the P0, P1, and P2 findings from the repository review without expanding the product feature set.

## 1. Goals

1. Restore the architecture fitness gate by reducing `fabric` root re-exports to the frozen maximum of 132 (`architecture-status.toml:68-70`).
2. Replace the security-reporting placeholder with GitHub Private Vulnerability Reporting (`SECURITY.md:13-17`).
3. Align the README capability and crate descriptions with the current workspace and real source paths (`Cargo.toml:1-22`, `README.md:181-239`).
4. Remove panic-based Robot harness construction and make unsupported construction a typed error (`crates/cognit/src/harness/mod.rs:67-83`).
5. Replace the fixed `MonoTime(0)` deadline check with an injected monotonic clock (`crates/executive/src/application/world_state.rs:93-104`).
6. Make the `proc` and `io` driver placeholders explicit non-production boundaries rather than implied implementations (`crates/corpus/src/drivers/proc/mod.rs:1`, `crates/corpus/src/drivers/io/mod.rs:1`).
7. Reduce maintenance risk in the three highest-risk large modules through responsibility-based extraction while preserving public behavior:
   - `crates/corpus/src/tools/tools/change_transaction.rs`
   - `crates/cognit/src/harness/linear/mod.rs`
   - `crates/executive/src/application/agent_control/mod.rs`
8. Unify the documented and enforced MSRV at Rust 1.85 and route repository Cargo commands through `scripts/cargo-agent.sh` (`Cargo.toml:24-28`, `CONTRIBUTING.md:13-40`, `.github/workflows/ci.yml:111-120`).
9. Strengthen releases with pre-package validation, checksums, SBOM/provenance metadata, and the repository Cargo wrapper (`.github/workflows/release.yml:46-69`, `.github/workflows/release.yml:117-123`).
10. Complete installed-runtime acceptance according to `AGENTS.md`, including system deployment, digest equality, stable systemd restart counters, and a real LLM request over the official user socket.

## 2. Non-goals

- Implement eBPF, FUSE, Android, embedded targets, local inference, or unfinished Phase 7/8 drivers.
- Change public runtime semantics merely to reduce file size.
- Raise architecture freeze limits to make the gate pass.
- Replace user-owned uncommitted changes in `config/architecture/`.
- Claim deployment acceptance from development binaries, temporary sockets, or isolated daemons.

## 3. Design

### 3.1 Architecture and security

Root-level `fabric` exports will be audited by external usage. At least four exports that do not justify the frozen compatibility surface will move to their owning submodule paths. Call sites will be updated rather than raising the freeze value.

Security reporting will direct reporters to the repository's GitHub Security Advisory form. Public issues remain explicitly prohibited. No unverified response-time promise or placeholder contact will remain.

### 3.2 Runtime correctness

`build_harness` will return a typed `Result` instead of panicking for a harness that requires Executive-owned ports. Callers must handle the construction error explicitly. Robot construction remains in Executive, preserving the existing dependency direction.

`WorldState` will receive or retain an `Arc<dyn fabric::Clock>`. Deadline evaluation will use `clock.mono_now()`. Tests will use a deterministic clock to verify both notification and expiry paths without wall-clock sleeps where practical.

The empty `proc` and `io` drivers will either be removed from compiled module exports when unused or documented and typed as unavailable capabilities. They will not pretend to be implemented production drivers.

### 3.3 Documentation truthfulness

The README crate table and dependency diagram will include every current workspace domain (`Cargo.toml:1-22`). Capability anchors will be replaced with paths that exist in the current tree. Vision-only claims will be labeled as vision or planned behavior; stable claims will be limited to implemented and tested paths.

A lightweight repository check will validate local Markdown links and capability code anchors so future moves cannot silently stale the README.

### 3.4 Large-module convergence

The large files will be split along existing responsibility boundaries, not rewritten:

```text
public module / orchestrator
  |-- state and lifecycle
  |-- validation and policy
  |-- execution adapters
  |-- receipts / settlement
  `-- tests and fixtures
```

Extraction order will follow dependency risk:

1. Pure data transformation and validation helpers.
2. Receipt/evidence construction.
3. I/O coordination with explicit inputs.
4. State-machine transitions only after characterization tests exist.

Existing public imports will remain stable where feasible through narrow re-exports inside the owning module, not at `fabric` crate root.

### 3.5 Toolchain and release

Rust 1.85 becomes the single declared MSRV. CI job names, toolchain invocations, README, and contribution instructions will agree. All repository build/test/check/lint/doc commands will use `bash scripts/cargo-agent.sh`.

The release flow will:

```text
checkout
  -> formatting / targeted release checks
  -> release build via cargo-agent.sh
  -> package
  -> SHA-256 manifest
  -> SBOM / provenance artifact
  -> GitHub release upload
```

The workflow will avoid network-time installation of optional Rust tooling where a deterministic metadata artifact can be generated from repository inputs. Release acceptance remains distinct from system-installed runtime acceptance.

## 4. Error handling

- Unsupported harness construction returns a typed error; it never panics.
- Clock acquisition remains infallible through the existing `Clock` contract.
- Documentation validation reports every missing path and exits non-zero.
- Release packaging fails if expected binaries, checksum files, or metadata are absent.
- Deployment or real-request failures are reported as failed acceptance, not downgraded to diagnostic success.

## 5. Verification

Use the narrowest checks during implementation, always through the repository wrapper:

1. Architecture: `bash scripts/aletheon.sh test architecture` and `bash scripts/aletheon.sh acceptance architecture`.
2. Cognit harness tests for typed construction failure.
3. Executive world-state tests for deterministic monotonic deadlines.
4. Corpus module checks for driver visibility and extracted transaction behavior.
5. Documentation-link and code-anchor validation.
6. Release workflow static validation and focused package checks.
7. Formatting: `bash scripts/cargo-agent.sh fmt --all -- --check`.
8. Integration owner only: workspace-wide tests if needed for cross-crate changes.
9. Final installed acceptance: `sudo bash scripts/aletheon.sh deploy`, SHA-256 equality across release/installed/running executables, stable restart counters, and a real LLM request through `/usr/bin/aletheon` on the official user socket.

## 6. Change isolation

The pre-existing edits in `config/architecture/executive-layers.tsv` and `config/architecture/metrics.env` are treated as user-owned. Implementation will inspect and accommodate them but will not revert them. Every staged change will be reviewed before any implementation commit.
