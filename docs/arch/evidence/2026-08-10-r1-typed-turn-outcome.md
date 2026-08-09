# Aletheon closure plan：R1 unified typed Turn Outcome

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §9
State: canonical typed Turn outcome established in Runtime; no compatibility path swallows errors

## Context receipt (runbook §3.1)

```text
Slice: R1 unified typed Turn Outcome
Baseline commit: 0bf690b2
Plan revision: closure-plan §9
Direct prerequisites: RA-04 (Turn reducer) — done
Current authoritative writer: unchanged legacy turn_engine/daemon_turn_engine/turn_pipeline
Target owner/writer: Runtime (canonical Turn outcome); not yet wired
IDs minted here: none — TurnId is Runtime-assigned
Production callers: none yet (types additive)
Test-only callers: 2 turn_outcome tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: legacy turn_engine → XRET-02
Unknowns/blockers: none
Expected files: crates/runtime/src/turn_outcome.rs, lib.rs re-export, R1 gate
Out-of-scope files: turn_engine/daemon_turn_engine/turn_pipeline cutover
```

## 1. What was created

`crates/runtime/src/turn_outcome.rs` (closure-plan §9 shape):

- **`StopReason`** (TurnComplete/MaxTurns/Interrupted), **`CancelReason`** (UserRequested/Timeout/Disconnect/SystemShutdown), **`BlockReason`** (ApprovalRequired/PolicyDenied/ResourceUnavailable).
- **`TurnFailure`** — typed, never swallowed.
- **`TurnOutcome`** — Completed/Cancelled/Blocked/Failed.
- **`TurnUsage`** — input/output/cache tokens.
- **`TurnExecutionResult`** — turn_id + outcome + usage + committed_event.
- **`TurnExecutionResult::terminal()`** — maps outcome to authoritative `TurnTerminal` (no inference path).

## 2. Rules honoured (R1)

- A turn's completion/stop/cancel/block/failure has unambiguous typed semantics (`TurnOutcome`).
- No compatibility path swallows an error or forges a permission: `TurnFailure` is typed; `terminal()` is authoritative.
- Reuses existing domain types where possible; the new shape follows §9's suggested form.
- Terminal only from typed outcome → `TurnTerminal`; no text/EOF inference (aligns RA-04).

## 3. R1 gate (architecture-check.sh)

`ARCH_SKIP_R1_GATES` requires `TurnOutcome`/`TurnExecutionResult`/`TurnUsage`; `TurnOutcome::Failed` must reference `TurnFailure` (no error swallowing).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib turn_outcome  PASS (2 passed)
  - completed_maps_to_terminal_completed
  - failed_maps_to_terminal_failed_with_message
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `turn_outcome.rs` + the lib.rs re-export + the R1 gate → exact baseline. No schema, no writer, no daemon change.

## 6. Next

R2 (per-turn OperationScope) and R3 (typed Command Output + protocol versioning) follow in the closure plan.
