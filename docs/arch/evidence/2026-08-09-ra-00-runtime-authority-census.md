# Agent Kernel V2：RA-00 Runtime authority census

Date: 2026-08-09
Baseline: `0bf690b2` (contains PR #192 foundation `1631ac3f`/`d9838d85`/`d3c5e748` + ratchet `f0aaec7e`/`7466dbfd`)
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §5
State: evidence-only census closed; `RA-00` gate added; no writer/schema/behavior change

## Context receipt (plan §3.1)

```text
Slice: RA-00 runtime authority census
Baseline commit: 0bf690b2 (contains PR #192 foundation)
Plan revision: docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md §5
Direct prerequisites: PR #192 foundation (F0 DONE)
Current authoritative writer: unchanged legacy Executive Session/Turn/AgentControl paths
Target owner/writer: unchanged by this census
IDs minted here: none (census records existing mints only)
Production callers: enumerated per row in runtime-authority-census.tsv
Test-only callers: counted separately, never as production authority
Installed/config callers: cross-checked against config/architecture/*.tsv
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: per-row cutover/deletion slice
Unknowns/blockers: none remain CLOSED; zero INVESTIGATE rows
Expected files: config/architecture/runtime-authority-census.tsv, this evidence record, architecture-check.sh gate
Out-of-scope files: all production behavior, all writer cutovers, RA-01+
```

## Census artifact

- `config/architecture/runtime-authority-census.tsv` — 60 rows, fixed 17 columns (plan §5.3), vocabularies verified:
  - `role` ∈ `id_mint | journal_writer | state_machine | composition | compatibility | registry | transport | reader | projection_writer` (all within §5.3 allowed set)
  - `authority_kind` ∈ `authority | compatibility | projection`
  - `status` = `CLOSED` on all 60 rows (0 `INVESTIGATE`)

## 1. Current authority summary (answers to §5.5)

### Q1/Q2 — who mints identity, and is it canonical/alias/endpoint/correlation?

| aggregate | mint | classification |
|---|---|---|
| Session | `RequestHandler::new` mints `uuid::Uuid::new_v4()` at `crates/aletheon/src/wiring/daemon/bootstrap/request.rs:81` | canonical, machine daemon created (RA-S-01) |
| Session | `SessionService::fork` mints child `SessionId(uuid::new_v4())` at `crates/runtime/src/session_service.rs:696` | canonical child (RA-S-02) |
| Session | `interact/src/single_message.rs:243` mints `SessionId(message-{uuid})` | client-side, must become Runtime receipt (RA-S-04) |
| Session | 17 interact/executive/aletheon reconstruction sites wrap daemon-returned strings as `SessionId` | legacy alias / wrapper, not mint (RA-S-05…RA-S-12) |
| Turn | `TurnCoordinator` mints `TurnId::new()` at `turn_coordinator.rs:610` | canonical (RA-T-01) |
| Turn | `TurnPipeline` derives/wraps `TurnId` from thread/operation ids (`turn_pipeline.rs:504,643,650,751,844,883,936`) | wrapper (RA-T-02) |
| Turn | TUI `reducer.rs:181,528` mints `TurnId` for live overlay | UI correlation → `UiOverlayId` (RA-T-04) |
| Turn | exec CLI (`main.rs:680`), exec_session (`composition/exec_session.rs:209`), launcher (`host/launcher.rs:408`) mint `TurnId` | exec envelope/wire (RA-T-06/07/08) |
| Agent | `AgentControlService::ValidatedAgentIdentity` mints `AgentId::new()` at `agent_control/mod.rs:811` | canonical (RA-A-01) |
| Agent | `stable_root_agent_id` uses `Uuid::new_v5` deterministic at `daemon_turn/lifecycle.rs:97` | canonical deterministic root (RA-A-02) |
| Agent | `AgentId` test fixtures in `memory.rs:268`, `live_runs.rs:253,254,463,464`, `settlement.rs:1385`, `admission.rs:56` | test-only (RA-A-03…06) |
| Delegate | no `AgentRunId`/`DelegateId` type exists; delegate identity is `AgentId` + `RuntimeId` keys | — |

### Q3 — who appends Session/Turn/Agent terminal?

- **Session**: `SessionService` writes `protocol_events` INSERT/UPDATE at `session_service.rs:105,134` (RA-S-14); `SessionStore` legacy `sessions` table (`store.rs:54,94,195`, RA-S-15); `CanonicalSessionStore` `sessions/session_items/session_principals` (`canonical_store.rs`, RA-S-16); `EventSourcedSessionStore` over `EventSpine` (`event_sourced_store.rs:292`, RA-S-17).
- **Turn**: `TurnCoordinator`/`TurnPipeline` are the turn state machines (RA-T-09/10); no separate durable turn journal — turn terminal lives through SessionAppendStore/EventSpine items.
- **Agent**: `SqliteAgentRunRepository` writes `agent_runs/agent_runtime_processes/agent_resource_leases/agent_messages_v2/agent_terminal_receipts` (`sqlite_repository.rs:130,210,377,488,574,706,736,794,1026`, RA-A-11).

### Q4 — is `SessionService.protocol_events` authoritative or Gateway reconnect log?

`protocol_events` is the daemon-to-client event projection table written by `SessionService::append_protocol_item_event` (`session_service.rs:76-134`); it is the **compat event log** for the client subscription path (`protocol_events_after_for` at `session_service.rs:378`, `handler/mod.rs:374`, `server.rs:462,654,690`). It is a projection/replay surface, not the canonical aggregate store; the canonical stream is `EventSpine`/`SessionAppendStore`. Target: Runtime `RuntimeJournal` (RA-02).

### Q5 — what does each journal/writer write?

| writer | stream/table | rows |
|---|---|---|
| `SqliteEventSpine` | `EventSpine` sqlite streams | RA-J-01 |
| `CanonicalEventBus` | in-memory notification (not durable journal) | RA-J-02 |
| `SessionService` | `protocol_events` | RA-S-14 |
| `SessionStore` (legacy) | `sessions` | RA-S-15 |
| `CanonicalSessionStore` | `sessions/session_items/session_principals` | RA-S-16 |
| `EventSourcedSessionStore` | `EventSpine` streams | RA-S-17 |
| `SqliteAgentRunRepository` | `agent_*` tables | RA-A-11 |

### Q6 — the two runtime registries

| registry | type | production construction | rows |
|---|---|---|---|
| `AgentRuntimeRegistry` | `RuntimeId → AgentRuntimeLauncher` | `request.rs:948`, `extensions.rs:463` | RA-A-07 |
| `RuntimeRegistry` (legacy) | `RuntimeId → SubAgentRuntime` | `orchestrator.rs:42` (`compatibility_runtimes`) | RA-A-08/09 |

Both hold Pi/Native launchers (RA-D-01/02/03). Target: single `DelegateBackendRegistry` at `RA-05`.

### Q7 — Native/Pi spawn/wait/cancel entry points

- Native: `NativeCognitRuntime` (`adapters/runtime/native_cognit.rs`), runtime_id at `:262`; registered as `AgentRuntimeRegistry` launcher at `request.rs:982`.
- Pi: `register_pi_runtime` at `adapters/runtime/pi.rs:66`; `PiRuntime::runtime_id()` at `:107`; `PiRuntime` config/resolution in `pi.rs`/`pi_rpc.rs`/`pi_protocol.rs`.
- Both are behind `AgentControlService::spawn` (`agent_control/mod.rs:1034`) / `AgentRuntimeLauncher` (`execution.rs:408`). Target `RA-05`/`E6-K6d`.

### Q8 — production/package/runtime manifest loaders

`MarkdownAgentProfileLoader` (`composition/agent_loader/mod.rs:49`) is the surviving production profile loader, composed at `host/daemon/bootstrap/runtime.rs:11,79,105,120`. PR #192 removed only `composition/agents`. Target: Runtime profile port (RA-05).

### Q9 — installed profile/config/schema references

No installed config or profile references retired authorities (`config/architecture/*.tsv` reference only live paths; `retired-authorities.tsv` shows the PR #192 deletions are zero-caller). `config/architecture/persistence-surfaces.tsv` still lists `agent-*` surfaces owned by `executive` — these become RA-02/RA-05 cutover inputs, not deletions.

### Q10 — old-test disposition

- `KEEP-EVIDENCE`: `session_projection.rs` projection tests, `sqlite_repository` migration/replay tests, `turn_recovery` typed-terminal tests (they encode current behavior).
- `REPLACE`: tests asserting `SessionService` minting behavior must move to Runtime receipts after `RA-03`.
- `DELETE-LATER`: tests directly constructing `LegacySessionService`/legacy `SessionStore` after `XRET-04`.
- No test is treated as production authority (all test-only rows carry `production_callers=0`).

## 2. Monotonic gate added

`scripts/libexec/aletheon/architecture-check.sh` now rejects:

- new unregistered `SessionId`/`TurnId`/`AgentId` `::new`/`new_v4`/`new_v5` mints under production paths not present in `runtime-authority-census.tsv`;
- new `Session-shaped`/`Turn-shaped`/`Agent-shaped` struct/enum/trait/type declarations not in the census;
- new `SessionAppendStore`/`AgentRunRepository`/`RuntimeRegistry`/`AgentRuntimeRegistry`/`EventSpine`/`CanonicalEventBus` constructor points not in the census;
- production `protocol_events`/`sessions`/`session_items`/`agent_*` INSERT/UPDATE/DELETE sites not in the census.

Registered rows may only decrease; the census path must exist (absence row requires a zero-match command).

## 3. Scope boundaries

- **Allowed** (this slice): census TSV, this evidence record, monotonic gate only.
- **Explicitly not done** (plan §5.6): no Runtime owner API, no `RA-01` contracts, no writer cutover, no production behavior change, no schema/table/migration change, no deployment.

## 4. Validation

Run per plan §5.7 (all PASS on 2026-08-09):

```text
git diff --check                                  PASS (no whitespace errors)
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS (syntax ok)
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
                                                  PASS: 23 findings, 0 dependencies, 4 paths; no additions
                                                  (includes the new RA-00 monotonic gate block)
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p runtime      PASS (Finished dev, 3.22s)
bash scripts/cargo-agent.sh check -p executive --lib  PASS (Finished dev, 4.24s)
```

### Gate mutation tests (probes, reverted after each)

| probe | result |
|---|---|
| new `AgentId::new()` in an uncovered file | REJECTED (unregistered authority site) |
| new `AgentId::new()` in a covered file | REJECTED (no matching census row) |
| drop a census row whose code remains | REJECTED (site unattributable) |
| corrupt a row to 16 columns | REJECTED (want 17) |
| change a CLOSED evidence regex to no-longer-matching | REJECTED (evidence drift) |
| clean baseline | ACCEPTED (74 rows, 17 cols, all evidence matches) |

## 5. Scope boundaries

- **Allowed** (this slice): census TSV, this evidence record, monotonic gate only.
- **Explicitly not done** (plan §5.6): no Runtime owner API, no `RA-01` contracts, no writer cutover, no production behavior change, no schema/table/migration change, no deployment.
- **Census counts**: 74 rows — `Session` 26, `Turn` 18, `Agent` 15, `Composition` 5, `Delegate` 5, `Journal` 5.
- **Test-only rows** (`production_callers=0`): RA-S-13, RA-T-03, RA-A-03/04/05/06 — counted separately, never as production authority.
