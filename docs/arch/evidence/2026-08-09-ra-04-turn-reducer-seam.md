# Agent Kernel V2：RA-04 canonical Turn reducer owner seam

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-04, runbook PR-A/PR-C
State: canonical Turn reducer seam established (PR-A); **writer cutover is PR-C deployment slice**

## Context receipt (runbook §3.1)

```text
Slice: RA-04 canonical Turn reducer/terminal fence (PR-A portion)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-04
Direct prerequisites: RA-03 (SessionAuthority) — done
Current authoritative writer: unchanged legacy TurnCoordinator/TurnPipeline
Target owner/writer: Runtime canonical Turn reducer; NOT yet the writer (PR-A seam)
IDs minted here: none — TurnId is Runtime-assigned (ids.rs)
Production callers: none yet (seam additive; legacy turn path untouched)
Test-only callers: 2 reducer tests (pure transition table)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a (TurnPipeline removal is RA-06/XRET-02)
Unknowns/blockers: none for the seam; the PR-C writer cutover is a separate deployment slice
Expected files: crates/runtime/src/turn_reducer.rs, lib.rs re-export, RA-04 gate
Out-of-scope files: the actual TurnCoordinator→Runtime cutover (PR-C, §14.2 deployment)
```

## 1. What was created

`crates/runtime/src/turn_reducer.rs`:

- **`TurnTransition`**: Start / Cancel / Timeout / Disconnect / LateReceipt / Crash / Settle — every path goes through one typed transition.
- **`TransitionOutcome`**: Terminal (authoritative) or Pending (with observation).
- **`TurnReducerSeam::apply`**: pure transition table — settle after terminal is typed `AlreadyTerminal` (never reversed); cancel/timeout/disconnect/crash/late-receipt after terminal are observation-only.

## 2. Rules honoured (plan §7 RA-04)

- **One Turn state machine / terminal writer**: the reducer is the single terminal fence contract.
- **Not moving the TurnPipeline God Object**: the reducer references no `TurnPipeline`/`TurnCoordinator` (RA-04 gate rejects both).
- **cancel/timeout/disconnect/late receipt/crash all go through typed transition**: covered by `TurnTransition` variants.
- **Valid late effect may only append observation/accounting, never reverse terminal**: `AlreadyTerminal` on duplicate settle; late paths return the preserved terminal or a Pending observation.
- **TUI core TurnId mint cleared by CGP-06** (separate slice, per plan).

## 3. RA-04 gate (architecture-check.sh)

`ARCH_SKIP_RA04_GATES` rejects any `TurnPipeline`/`TurnCoordinator` reference in the reducer code; requires the `AlreadyTerminal` terminal fence.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib turn_reducer  PASS (2 passed)
  - settle_after_terminal_is_rejected_no_reversal
  - cancel_after_terminal_is_observation_only
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-04 PR-C (separate deployment slice, §14.2)

The actual Turn writer cutover — one reducer/terminal fence, cognition/context/policy/effect/post-settle as narrow ports, TUI `UiOverlayId`, post-settle outbox, installed Native multi-turn acceptance + rollback binary — is a **deployment-gated slice** (deploy, SHA-256, systemd restart counter, real LLM multi-turn, cancel/timeout/crash drills).

## 6. Rollback

Delete `turn_reducer.rs` + lib.rs re-export + RA-04 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

RA-05 (AgentSupervisor + DelegateBackendRegistry seam) continues the strict main-writer chain; each PR-C cutover is independently deployed.
