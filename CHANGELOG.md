# Changelog

All notable changes to the Aletheon runtime are documented in this file.

The authoritative release evidence for any tagged release is the
machine-generated acceptance scoreboard bound to the immutable RC,
not this changelog. See `docs/release/known-limitations.md` for
current known limitations.

## [Unreleased]

### Added
- Release workflow source identity gate: semver tag validation, explicit
  `origin/main` and `origin/dev` ref fetch, origin/main commit equality,
  origin/dev ancestor-of-main check.
- Workspace-level `fmt`, `clippy --workspace --all-targets --all-features`,
  `test --workspace --all-features`, architecture check, and
  migration-matrix verification in the release validate job.
- Mandatory simulation Robot R8 evidence gate: the release fails closed
  unless fresh positive (completed) and negative (safe-stop) receipts are
  downloaded and verified for the exact GITHUB_SHA and RC digest.
- Two-job R8 evidence handoff workflow (`r8-evidence-handoff.yml`) with
  protected operator-pause environment for self-hosted simulation acceptance.
- Python R8 evidence verifier (`tests/coding/harness/r8_evidence_verifier.py`)
  with metadata binding, archive/installed/receipt digest cross-checks, and
  deterministic unit tests.
- Post-release smoke job consuming the published x86_64 release asset with
  exact SHA256SUMS line match (not `--ignore-missing`) and `aletheon
  version --json` commit-binding validation.
- Machine-readable smoke receipt with tag, commit, binary digest, acceptance
  run ID, R8 receipt digests, and actual previous-tag rollback point.
- Machine-generated release notes requiring (not optionally omitting)
  acceptance run ID, installed digest, R8 positive/negative receipt
  digests, and R8 evidence archive digest; first-release handling without
  malformed compare URL.
- `docs/release/known-limitations.md` documenting all known limitations.
- Per-turn operation scopes with bounded cancellation/deadline cleanup and
  exactly-once terminal settlement (R2).
- Versioned typed command output and a canonical live/durable TUI reducer
  (R3/U1).
- Machine-generated installed acceptance manifests and scoreboards (A1).
- A reproducible 30-task synthetic Robot Engineering benchmark with
  deterministic independent oracles (E1).
- Indexed per-session event append, bounded conflict retries, and the
  1k/10k/100k by 1/10/100-session benchmark matrix (S1).
- Dependency-aware read-only tool caching, configurable recall caching, and
  observable prompt/tool profiles (C1).
- A bounded evidence-driven role workflow with explicit transition reasons
  and repair-loop limits (M1).

### Known Limitations (Unresolved)
- Physical HIL (H1): Not performed. Simulation R8 is the highest Robot
  evidence accepted by the release workflow; no physical-support claim is made.
- Release promotion: `dev` to `main`, immutable RC acceptance, semantic tag,
  and post-release smoke remain release-time operational gates; this unreleased
  entry does not assert they have run.
- Live post-stage positive/negative Robot tasks, tag/publish/smoke, and
  environment protection/configuration remain operator-driven gates.

## [0.1.0] - 2026-06-06

### Added
- Initial release of Aletheon agent runtime
- Core architecture: Nous (Soul/Brain/Body) triune design
- Crate structure: 10 crates organized by domain
- ReAct cognitive engine with reasoning, planning, and reflection
- Memory system: episodic, semantic, and procedural memory
- Security model: L0-L3 permission tiers
- Linux platform integration: eBPF, FUSE, systemd
- Android platform design: AccessibilityService, Foreground Service
- Embedded support: RK3588, Jetson Orin Nano
- CLI and TUI interface
- MCP server definitions
- Agent definition system (TOML + Markdown)
