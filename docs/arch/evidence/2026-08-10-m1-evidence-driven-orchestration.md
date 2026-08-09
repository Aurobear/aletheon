# Aletheon closure plan：M1 evidence-driven multi-agent controller

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §17
State: evidence-driven orchestration stage machine established; fixed role pipeline not yet cut over

## Context receipt (runbook §3.1)

```text
Slice: M1 evidence-driven multi-agent controller (stage-machine seam)
Baseline commit: 0bf690b2
Plan revision: closure-plan §17
Direct prerequisites: R0/X0/R1/R2/R3 + U1/A1 (per plan); R1/R2/R3 done here
Current authoritative writer: unchanged legacy 2k-line orchestration (fixed role pipeline)
Target owner/writer: Runtime orchestration seam; not yet wired
IDs minted here: none
Production callers: none yet (stage machine additive)
Test-only callers: 3 orchestration tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam
Expected files: crates/runtime/src/orchestration.rs, lib.rs re-export, M1 gate
Out-of-scope files: the 2k-line orchestration split, risk classifier wiring, U1/A1
```

## 1. What was created

`crates/runtime/src/orchestration.rs`:

- **`TaskRisk`** — Low / Medium / High (drives stage skipping).
- **`EvidenceGap`** — NoVerification / NoReview / NoRepro / None (Tester/Reviewer/Fixer triggers).
- **`Stage`** — Planner / Explorer / Executor / Tester / Reviewer / Fixer.
- **`TransitionReason`** — RiskLowSkip / EvidenceGap / FailureEvidence / LoopCapReached / Complete.
- **`OrchestrationStep`** — Run(stage) / Skip(stage) / Settle.
- **`EvidenceDrivenController::next`** — pure decision machine.

## 2. Rules honoured (M1)

- **Low-risk tasks skip Planner/Explorer**: `RiskLowSkip` (test-verified).
- **Tester/Reviewer trigger on missing evidence, not fixed role counts**: `EvidenceGap::NoVerification`/`NoReview` → Tester/Reviewer (test-verified).
- **Fixer only receives structured failure evidence, loop-capped**: `fix_loop_cap` → `LoopCapReached` settle (test-verified).
- **All roles share one OperationScope/permission/terminal**: documented; settles via the single reducer (RA-04) — M1 gate requires `Settle` + single settlement path.
- **No new top-level crate**: the seam lives in `crates/runtime/src/orchestration.rs`.
- **All transitions have a reason code**: `TransitionReason`.

## 3. M1 gate (architecture-check.sh)

`ARCH_SKIP_M1_GATES` requires `EvidenceGap`/`fix_loop_cap`/`RiskLowSkip`/`TransitionReason`; and that the machine settles to the single reducer (no second settlement path).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib orchestration  PASS (3 passed)
  - low_risk_skips_planner
  - tester_reviewer_triggered_by_evidence_gap_not_fixed_count
  - fixer_is_capped
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining M1 (cutover)

Split the 2k-line orchestration into private stages, wire the risk classifier + controlled parallelism, trigger Tester/Reviewer on live evidence, and route all roles through the single OperationScope/permission/settlement. Gated on U1/A1 per plan §17.

## 6. Rollback

Delete `orchestration.rs` + the lib.rs re-export + the M1 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

A1 (installation scoreboard), E1 (robot benchmark), H1 (physical HIL) remain in the closure plan; U1 is the in-flight TUI branch.
