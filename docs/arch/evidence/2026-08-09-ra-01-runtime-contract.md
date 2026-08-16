# Agent Kernel V2：RA-01 Runtime commands/events/queries/ID owner

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §11.2
State: Runtime owner contract established; **no writer cutover** (contract/port only)

## Context receipt (runbook §3.1)

```text
Slice: RA-01 Runtime commands/events/queries/ID owner
Baseline commit: 0bf690b2
Plan revision: implementation-plan §11.2
Direct prerequisites: RA-00 (census), D0/D1 (contracts seed) — done
Current authoritative writer: unchanged legacy Executive Session/Turn/AgentControl paths
Target owner/writer: Runtime (canonical IDs + commands/events/queries/ports defined; not yet wired)
IDs minted here: none — IDs are defined as types; the Runtime composition mints them at RA-03/04/05
Production callers: none yet (contract modules additive; legacy path untouched)
Test-only callers: none
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/runtime/src/{command,event,query,ids,error,ports}.rs, lib.rs, Cargo.toml
Out-of-scope files: all writer cutovers, RA-02+, Executive behavior
```

## 1. What was created (plan §11.2 target files)

| file | content |
|---|---|
| `ids.rs` | Runtime canonical `SessionId(String)`, `TurnId(String)`, `AgentRunId(String)`, `Generation(u64)` — assigned by Runtime composition |
| `command.rs` | `CreateSessionCommand` (no aggregate ID accepted), `ResumeSessionCommand`, `StartTurnCommand`, `SpawnAgentRunCommand`, `CancelTurnCommand`, `CommandReceipt` |
| `event.rs` | `RuntimeEvent` (SessionCreated/TurnStarted/TurnSettled/AgentRunStarted/AgentRunSettled) with authoritative `TurnTerminal` |
| `query.rs` | `SessionSnapshotQuery`, `AgentRunQuery`, `RuntimeQuery` |
| `error.rs` | `RuntimeError` typed failures (SessionNotFound/WrongGeneration/AlreadyTerminal/Timeout/ProviderRejected/ConnectionClosed/UnknownSchema/Internal) |
| `ports.rs` | `RuntimeCommandPort`, `RuntimeQueryPort`, `RuntimeEventPort` narrow traits |

## 2. Rules honoured (plan §11.2)

- `CreateSessionCommand`/`StartTurnCommand`/`SpawnAgentRunCommand` **accept no new aggregate ID** — only correlation references; the Runtime assigns canonical IDs (`ids.rs` is the single ID source).
- Caller correlation and `LegacySessionAlias` carry **no authority** (documented in ids.rs).
- ID source only in Runtime composition (the ids module is the only definition site — RA-01 gate enforces this).
- **Contract/port only**: no append, no spawn, no legacy-writer cutover.
- `UiOverlayId` never enters the Runtime (Presentation-local, per interact census).
- Terminal only from typed `TurnSettled`/`AgentRunSettled` events — a client never infers terminal from text/EOF/spinner.

## 3. Boundary discipline

- `runtime` depends only on `serde`/`serde_json`/`async-trait`/`thiserror` — **no Executive**, no interact, no gateway, no corpus/kernel (RA-01 gate rejects any such dependency).
- Canonical aggregate-ID `struct` definitions only in `ids.rs` (RA-01 gate rejects elsewhere).
- Runtime canonical `SessionId`/`TurnId` registered in `id-collisions.tsv` as **retain-distinct-semantics** vs fabric legacy shared IDs, with exit nodes `RA-03`/`RA-04`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime      PASS (0.39s)
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
  - module-boundaries runtime row updated to command,error,event,ids,manifest,ports,query,selector
  - id-collisions.tsv registers Runtime SessionId/TurnId distinct semantics
```

## 5. Rollback

Delete the six runtime modules + lib.rs re-exports + Cargo.toml thiserror + boundary/id-collisions rows → exact baseline. No wire tag, schema, writer, or daemon touched.

## 6. Next

RA-01 done → **RA-02** (durable RuntimeJournal shadow), **K1** (durable Operation authority) unblocked on the Runtime-contract axis. CGP-01 composition skeleton can now consume Runtime `RuntimeCommandPort`/`RuntimeEventPort`.
