# Agent Kernel V2：RA-04 PR-C Runtime Turn writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-04, runbook PR-C
State: deployable Runtime Turn writer built; legacy TurnCoordinator stays authoritative until the PR-C deployment

## Context receipt (runbook §3.1)

```text
Slice: RA-04 PR-C canonical Turn writer (code)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-04, runbook §7.3 PR-C
Direct prerequisites: RA-04 PR-A (Turn reducer) + R1 (typed outcome) — done
Current authoritative writer: unchanged legacy TurnCoordinator/TurnPipeline
Target owner/writer: RuntimeTurnWriter; not yet wired
IDs minted here: none — the writer mints at runtime
Production callers: none yet (writer additive)
Test-only callers: 2 turn_writer tests
Installed/config callers: none (not yet deployed)
Tables/files/wire schemas: none changed
External side effects: none (no deploy yet)
Compatibility seam: none (PR-B shadow is the deploy step)
Deletion owner: legacy TurnPipeline → RA-06/XRET-02
Unknowns/blockers: the installed acceptance (deploy + real LLM multi-turn + rollback drill) is the deployment slice
Expected files: crates/runtime/src/turn_writer.rs, lib.rs re-export, RA-04C gate
Out-of-scope files: the daemon wiring + deployment
```

## 1. What was created

`crates/runtime/src/turn_writer.rs`:

- **`RuntimeTurnWriter`** — mints the canonical `TurnId`, starts a turn through the single `TurnReducerSeam` fence, settles to a typed terminal, and emits `TurnStarted`/`TurnSettled` events. Implements `RuntimeCommandPort` for `StartTurn`.

## 2. Rules honoured (RA-04 PR-C)

- **Runtime assigns the TurnId** (`TurnId(format!("turn-{session}"))`) — no caller-minted ID.
- **Single reducer/terminal fence**: every start/settle goes through `TurnReducerSeam`; no second terminal state machine (RA-04C gate rejects `TurnPipeline`/`TurnCoordinator` in the writer).
- **cancel/timeout/disconnect/late receipt/crash all typed**: the reducer's transition set covers them; `settle` uses the fence.
- **Valid late effect only appends observation, never reverses terminal**: a duplicate settle returns `AlreadyTerminal` (test-verified; still one `TurnSettled`).
- Legacy writer stays authoritative until PR-C deployment.

## 3. RA-04C gate (architecture-check.sh)

`ARCH_SKIP_RA04C_GATES` requires `RuntimeTurnWriter`/canonical `TurnId` mint/`TurnReducerSeam`/`AlreadyTerminal`; rejects `TurnPipeline`/`TurnCoordinator`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib turn_writer  PASS (2 passed)
  - writer_mints_turn_and_settles_through_the_single_fence
  - duplicate_settle_is_rejected_no_terminal_reversal
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-04 PR-C (deployment)

Wire into the daemon (gated, like RA-03), then maintenance/drain + freeze + switch, `sudo deploy` + SHA-256 compare + systemd restart counter + real LLM multi-turn (model-controlled route/arguments three times per plan) + cancel/timeout/crash drills + rollback binary. Per plan §12.3, must not merge RA-03/04/05.

## 6. Rollback

Delete `turn_writer.rs` + the lib.rs re-export + the RA-04C gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

RA-05 PR-C (AgentSupervisor) follows on the strict main-writer chain.
