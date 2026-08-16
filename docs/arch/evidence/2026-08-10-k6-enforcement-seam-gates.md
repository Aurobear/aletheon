# Agent Kernel V2：K6 Kernel enforcement seam gates

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-kernel-enforcement-consolidation.md` §10 K6
State: enforcement seam gates delivered; extension joint cutovers (E2-K6a/E4-K6b/E5-K6c/E6-K6d) consume them; no extension migration

## Context receipt (runbook §3.1)

```text
Slice: K6 Kernel seam/gates ready for extension joint cutovers
Baseline commit: 0bf690b2
Plan revision: kernel-enforcement §10 K6
Direct prerequisites: K0..K5 — done
Current authoritative writer: unchanged legacy Executive extension paths
Target owner/writer: Kernel enforcement seam (gates only); extensions switch via their unique owner PRs
IDs minted here: none
Production callers: none yet (gate module additive)
Test-only callers: 2 enforcement tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam
Expected files: crates/kernel/src/enforcement.rs, kernel lib.rs, K6 gate, metrics.env baseline update
Out-of-scope files: all extension cutovers (E2-K6a/E4-K6b/E5-K6c/E6-K6d), K7
```

## 1. What was created

`crates/kernel/src/enforcement.rs`:

- **`EnforcementFamily`** — Gmail / Hardware / RobotVla / Pi.
- **`FamilyGate`** — `OwnerCutoverRequired(family)` / `LegacyAuthoritative`.
- **`UniquenessViolation`** — DuplicateWriter / DuplicateExecutor / ShadowEffect.
- **`EnforcementSeamGate`** — per-family writer/executor registration with uniqueness enforcement.

## 2. Rules honoured (K6)

- **Stable Kernel enforcement seam + per-family registration gate + writer/executor uniqueness gate**: `EnforcementSeamGate` (duplicate writer/executor rejected).
- **Cutover runbook**: each unique owner PR (E2-K6a / E4-K6b / E5-K6c / E6-K6d) drains active operations, records watermark/registry revision/authority generation, and does the end-to-end cutover — the joint number is one PR satisfying both plans, not two switches (documented).
- **Kernel does not migrate extensions**: the gate only coordinates; no extension transport/adapter import (K6 gate rejects extension-driving code).
- **Domain-owned veto/outbox/terminal evidence not replaced by Kernel receipts**: documented; a failed shard returns to legacy-authoritative.

## 3. Metric baseline update (reviewed)

`CORE_EXTERNAL_IDENTIFIER_HITS` 21→23: the K6 seam legitimately introduces extension-family labels (`Gmail`, `Pi`) required by plan §K6's cutover numbering. This is a reviewed monotonic-ratchet increase (same pattern as the R0/X0 freeze update), not a provider leak into application code. Updated in `config/architecture/metrics.env` with this evidence record.

## 4. K6 gate (architecture-check.sh)

`ARCH_SKIP_K6_GATES` requires DuplicateWriter/DuplicateExecutor/ShadowEffect; rejects extension-driving code (`gmail_*`/`hardware_*`/`robot_*`/`pi_*` with transport/adapter/send/spawn) in the seam.

## 5. Validation

```text
bash scripts/cargo-agent.sh check -p kernel        PASS
bash scripts/cargo-agent.sh test -p kernel --lib enforcement  PASS (2 passed)
  - duplicate_writer_is_rejected
  - family_decision_requires_owner_cutover
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps, baseline 21→23)
```

## 6. Rollback

Delete `enforcement.rs` + the kernel lib.rs module line + the K6 gate; revert metrics.env 23→21 → exact baseline. No schema, no writer, no daemon change.

## 7. Next

K7 (Executive path deletion + Kernel surface contraction) is the cleanup; the E-series joint cutovers consume this seam.
