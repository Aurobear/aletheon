# Agent Kernel V2：RA-04 PR-C Runtime Turn writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-04, runbook PR-C
State: Runtime Turn writer is wired as the canonical terminal fence and durable TurnStream writer; TurnCoordinator remains the execution/projection facade

## Context receipt (runbook §3.1)

```text
Slice: RA-04 PR-C canonical Turn writer (code)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-04, runbook §7.3 PR-C
Direct prerequisites: RA-04 PR-A (Turn reducer) + R1 (typed outcome) — done
Current authoritative writer: RuntimeTurnWriter for TurnId and terminal; TurnCoordinator remains host execution and SessionAppendStore projection
Target owner/writer: RuntimeTurnWriter + TurnStream sink
IDs minted here: Runtime-owned UUID TurnId
Production callers: TurnCoordinator starts and settles through RuntimeTurnWriter; terminal is fenced before legacy item projection
Test-only callers: runtime turn_writer invariant/recovery tests
Installed/config callers: daemon build_turn_services injects the writer and replays TurnStream
Tables/files/wire schemas: `aletheon.event.runtime_turn/v1` versioned TurnStream envelope
External side effects: durable EventSpine append; legacy SessionAppendStore remains projection
Compatibility seam: TurnCoordinator translates execution outcome/cancel/disconnect into Runtime terminal/observation commands
Deletion owner: legacy TurnPipeline → RA-06/XRET-02
Unknowns/blockers: official deploy/client smoke is green, but the installed acceptance (real LLM multi-turn + cancel/timeout/crash + rollback drill) remains open
Expected files: crates/runtime/src/turn_writer.rs, lib.rs re-export, RA-04C gate
Remaining scope: maintenance/drain evidence, installed multi-turn and failure drills, and rollback-binary verification
```

## 1. What was created

`crates/runtime/src/turn_writer.rs`:

- **`RuntimeTurnWriter`** — mints the canonical `TurnId`, starts a turn through the single `TurnReducerSeam` fence, settles to a typed terminal, and emits versioned `TurnStreamEvent` records. Cancel/timeout/disconnect/crash/late receipt are observation transitions through the same reducer.
- Terminal append/publication is serialized so concurrent completion,
  disconnect, and recovery paths cannot all pass the in-memory fence before a
  journal append completes (`crates/runtime/src/turn_writer.rs:21-31,113-120`).
- The coordinator's dropped-future guard records a typed disconnect or crash
  observation before its interrupted/failed terminal
  (`crates/executive/src/application/turn_coordinator.rs:206-262`).

## 2. Rules honoured (RA-04 PR-C)

- **Runtime assigns the TurnId** (`TurnId` UUID) — no caller-minted ID.
- **Single reducer/terminal fence**: every start/settle goes through `TurnReducerSeam`; no second terminal state machine (RA-04C gate rejects `TurnPipeline`/`TurnCoordinator` in the writer).
- **cancel/timeout/disconnect/late receipt/crash all typed**: the reducer's transition set covers them; `settle` uses the fence.
- **Valid late effect only appends observation, never reverses terminal**: a duplicate settle returns `AlreadyTerminal` (test-verified; still one `TurnSettled`).
- The legacy coordinator writes only compatibility projection items after the Runtime terminal fence; it no longer wins terminal ordering.

## 3. RA-04C gate (architecture-check.sh)

`ARCH_SKIP_RA04C_GATES` requires `RuntimeTurnWriter`/canonical `TurnId` mint/`TurnReducerSeam`/`AlreadyTerminal`; rejects `TurnPipeline`/`TurnCoordinator`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib  PASS (55 passed; TurnWriter/Reducer/Session writer invariants included)
  - writer_mints_turn_and_settles_through_the_single_fence
  - duplicate_settle_is_rejected_no_terminal_reversal
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
bash scripts/cargo-agent.sh test -p executive --test turn_coordinator_lifecycle  PASS (14 passed)
```

## 5. Remaining RA-04 cutover/acceptance

`sudo bash scripts/aletheon.sh deploy` plus SHA-256 parity, stable systemd counters, and an official client request have passed. Complete maintenance/drain + freeze evidence, real LLM multi-turn (model-controlled route/arguments three times per plan), cancel/timeout/crash drills, and rollback-binary verification. The writer is already wired, but RA-04 is not closed until this installed acceptance and legacy caller-zero audit pass.

Continuation (2026-08-11): the latest deployment after the Interact memory-client cutover
passed with installed/release SHA-256
`7ab5feeac2b0e94cfede2c3f29f0a220f0e1f57b9da8e2f15ac6c0825a0a3b61`; the official client
request and service-stability checks passed. A subsequent scheduled Agent fixture failed
closed with `typed Gateway turn did not reach a durable terminal projection`, so the
multi-run terminal acceptance remains open rather than being reported as success.

Latest installed observation (2026-08-10 15:07 UTC):
`sudo bash scripts/aletheon.sh deploy` passed; `target/release/aletheon`,
`/usr/bin/aletheon`, the machine-core executable, and the user-daemon executable
all resolved to SHA-256
`3538dadb745daba6c4ce8956359770f9cc7aeba23c55c0999319ad7e771b791d`.
`aletheon-core.service` and the user `aletheon.service` were active with
`NRestarts=0`; `/usr/bin/aletheon exec --output json` returned a real terminal
receipt with `status=completed`, output `OK`, one iteration, and zero provider
retries. This does not replace the required multi-turn/failure/rollback matrix.

Continuation (2026-08-11): the typed TUI now correlates terminal projection to
the Runtime-assigned `TurnId` and immediately redraws after a projection
mutation. The latest official two-turn run rendered both terminal answers with
no stale spinner, and its typed session snapshot contained completed steps.
The installed artifact and running daemon executables matched SHA-256
`16b4e9284e2a1adeb6e1f1bad8e65a8a817d43b6b409fd3c2a772d8eb3e08eee`.
This is presentation/receipt evidence only; RA-04 remains open until the
installed cancel, timeout, disconnect/crash recovery, late-receipt, and
rollback-binary matrix is captured.

The latest installed timeout probe used the official `/usr/bin/aletheon` and
official user socket with `--timeout-seconds 1`; it returned a durable terminal
receipt with `status=cancelled`, `error_code=deadline_exceeded`, zero tool calls,
zero provider retries, and `completed_normally=false`. This is one real timeout
receipt, not completion of the full cancel/disconnect/crash/restart matrix.

## 6. Rollback

Rollback requires draining the Runtime turn writer and starting a binary that can read the additive `runtime_turn/v1` envelope; do not run old and new writers concurrently. The old coordinator projection remains readable during rollback.

## 7. Next

RA-05 PR-C (AgentSupervisor) follows on the strict main-writer chain.

The 2026-08-11 installed TUI recheck also rendered two completed turns without a
stale spinner (`/tmp/tui-current-18581.frames.jsonl`). It is presentation
evidence and does not close the required crash/restart, duplicate/late receipt,
generation, reconciliation, or rollback-binary acceptance matrix.

2026-08-11 replay integrity follow-up: Runtime Turn replay now fails closed on a
`TurnSettled` event without a matching `TurnStarted`, conflicting duplicate
terminal events, or conflicting duplicate start sessions. The focused Turn writer
suite passes 11 tests; Runtime/Executive checks, format, architecture acceptance,
and diff check remain green.

Replay validation was extended to bind each terminal to the same SessionId as
its start event; a cross-session terminal is rejected as `UnknownSchema`. The
Turn writer focused suite is now 12 passing tests.

TurnStream replay now also rejects duplicate/non-monotonic logical sequence
numbers before applying lifecycle events. The focused Turn writer suite is now
13 passing tests.

Turn command writes are now session- and turn-fenced: a settle or lifecycle
transition for another SessionId returns typed `WrongSession`, and a transition
whose embedded TurnId differs from the call boundary returns `TurnNotFound`.
Focused Turn writer evidence is now 14 passing tests.

### 2026-08-12 Runtime Turn lifecycle port cutover

Runtime now owns the narrow `TurnLifecycleWriter` port in
`crates/runtime/src/ports.rs`. Executive `TurnCoordinator` consumes the port as
`Arc<dyn runtime::TurnLifecycleWriter>`; the concrete Runtime writer, reducer,
replay state, and sink remain behind the Runtime boundary. The port covers
start, settle, cancel, timeout, disconnect, and crash. Runtime (92 tests) and
TurnCoordinator lifecycle (14 tests) passed, with architecture and format/diff
checks green. The reducer is now injected through a narrow port, but the
TurnCoordinator execution façade and final XRET cleanup remain open.

Installed verification for the lifecycle-port follow-up passed at SHA
`62e9dbd4273b73c18b248779828604473ee6e48e243aaccaf5f69268874dcd2d`.
`target/release/aletheon`, `/usr/bin/aletheon`, and both running daemon
executables matched; official client and Memory Agent smokes passed, with
core/user services active, `ExecMainStatus=0`, and `NRestarts=0`.

### 2026-08-12 Runtime Turn cancellation ownership cutover

The first-reason-wins cancellation controller moved from Executive
`TurnCoordinator` into `crates/runtime/src/turn_cancellation.rs`. Runtime owns
both the typed `fabric::CancelReason` receipt and cancellation token; Executive
retains only host orchestration and calls `TurnCancellation::request` during
user/deadline/disconnect/shutdown paths. Runtime cancellation tests and the
14-test TurnCoordinator lifecycle suite passed. The reducer remains the sole
terminal fence; the execution façade and final XRET gates remain open.

Installed verification for the cancellation cutover passed at SHA
`83356c0bc156411781317f9232b955eed70a0f40b85b56a771eda741d9689a48`.
All four release/runtime executable digests matched; official client and
Memory Agent smokes passed, with core/user services active and `NRestarts=0`.

### 2026-08-12 Typed late-receipt lifecycle port

`TurnLifecycleWriter` now includes an explicit `late_receipt` operation. The
Runtime implementation routes it through `TurnReducerSeam`, preserving
terminal state and journaling observation-only semantics. Runtime (93 tests),
TurnCoordinator lifecycle (14 tests), Executive check, architecture, and
diff/format checks passed.

Installed verification for the typed late-receipt lifecycle-port follow-up
passed at SHA `942985d15281816cb464e10cbf291de056df0b17ab33ad780cd22abea4b0c140`.
All four executable digests matched; official client and Memory Agent smokes
passed, with core/user services active and `NRestarts=0`.

### 2026-08-12 Runtime active-Turn registry ownership cutover

`ActiveTurn` and `ActiveTurnKey` moved from Executive
`application::turn_coordinator` to `crates/runtime/src/turn_registry.rs`.
`ActiveTurn` carries both the Runtime canonical TurnId and the Fabric wire
projection, plus the Runtime-owned cancellation record. Executive keeps only
the host active index and compatibility re-export. Runtime (93 tests),
TurnCoordinator lifecycle (14), Executive check, and format/diff checks passed;
the execution façade remains open for final RA-04/XRET cleanup.

Installed verification for the active-turn registry follow-up passed at SHA
`bb50ed09d3d5eb7d6efa406043aaed960b933bff716198ff98972e1ce76254e1`; all four
executable digests matched, official client/Memory Agent smokes passed, and
core/user services remained active with `NRestarts=0`.

### 2026-08-12 Runtime active-Turn registry ownership cutover

`ActiveTurn` and `ActiveTurnKey` moved from Executive
`application::turn_coordinator` to `crates/runtime/src/turn_registry.rs`.
`ActiveTurn` carries both the Runtime canonical TurnId and the Fabric wire
projection, plus the Runtime-owned cancellation record. Executive keeps only
the host active index and compatibility re-export. Runtime (93 tests),
TurnCoordinator lifecycle (14), principal/session focused tests (4), Executive
check, and format/diff checks passed; the execution façade remains open for
final RA-04/XRET cleanup.
