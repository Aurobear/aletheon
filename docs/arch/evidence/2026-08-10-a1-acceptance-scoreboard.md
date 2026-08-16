# Aletheon closure plan：A1 installation acceptance scoreboard

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §13
State: single machine-generated scoreboard model established; release judgment depends only on it

## Context receipt (runbook §3.1)

```text
Slice: A1 installation acceptance scoreboard
Baseline commit: 0bf690b2
Plan revision: closure-plan §13
Direct prerequisites: none (standalone evidence model)
Current authoritative writer: n/a (new scoreboard model)
Target owner/writer: aletheon host (scoreboard producer at release gate)
IDs minted here: none
Production callers: none yet (model additive)
Test-only callers: 3 scoreboard tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/aletheon/src/scoreboard.rs, lib.rs mod, A1 gate
Out-of-scope files: the run harness that emits artifacts/acceptance/<run-id>/*
```

## 1. What was created

`crates/aletheon/src/scoreboard.rs`:

- **`AcceptanceStatus`** — NotRun / InfraBlocked / Failed / Passed / Waived (the only allowed states).
- **`Waiver`** — approver + reason + expiry (cannot be default for a P0 gate).
- **`TaskResult`** — status + timing + exit_code + evidence_path + optional waiver.
- **`BuildFacts`** — repo SHA + dirty + profile + features.
- **`InstalledFacts`** — artifact digest (not just source SHA) + daemon/protocol version + redacted provider endpoint.
- **`Scoreboard`** — run_id + build + installed + tasks + scope_leak_count + retry_count + generation_id.
- **`Scoreboard::overall_passed(p0_gates)`** — release judgment: P0 gates must be Passed (waived fails).

## 2. Rules honoured (A1)

- **Single machine-generated evidence**: the scoreboard is the only release-judgment source.
- **Only the allowed status enum**: prose states (`code_complete`/`looks_good`/`evidence_complete`) are rejected (A1 gate; compile-time by enum).
- **`waived` requires approver + reason + expiry** (`Waiver`).
- **`waived` cannot be used for a P0 gate** (`overall_passed` rejects waived P0 — test-verified).
- Installed artifact digest, not just source SHA (`InstalledFacts.artifact_digest`).
- Scope/resource leak count + retry count + generation id recorded.

## 3. A1 gate (architecture-check.sh)

`ARCH_SKIP_A1_GATES` requires `AcceptanceStatus`/`Waiver`/`overall_passed`; rejects prose states in production code (doc comments that mention them only to reject are exempt).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p aletheon      PASS (0 warnings)
bash scripts/cargo-agent.sh test -p aletheon --lib scoreboard  PASS (3 passed)
  - status_enum_rejects_prose_states
  - all_p0_passed_returns_ok
  - waived_p0_gate_fails
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining A1 (harness)

The run harness that emits `artifacts/acceptance/<run-id>/{manifest,scoreboard}.json|md` and `logs/`, wired into the release gate.

## 6. Rollback

Delete `scoreboard.rs` + the lib.rs mod + the A1 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

E1 (robot benchmark) and H1 (physical HIL) are deployment/HIL-gated; REL (dev promotion) consumes this scoreboard.
