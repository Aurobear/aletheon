# Agent Kernel V2：RA-02 shadow/replay evidence closure

Date: 2026-08-10
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §12.1, rework brief §5.2 first item
State: **evidence closed** — shadow reads the real production event spine read-only; writer cutover remains PR-C deployment

## Context receipt

```text
Slice: RA-02 shadow/replay evidence closure
Baseline commit: 0bf690b2..d0286ca7 (rework target)
Real baseline: production user daemon (PID 102238) holds 23229 spine events in
  ~/.local/state/aletheon/events.db (verified via /proc fd + sqlite3)
Current authoritative writer: unchanged legacy SessionAppendStore/SqliteEventSpine
Target owner/writer: Runtime RuntimeJournalShadow (read-only replay; writer cutover is PR-C)
IDs minted here: none
Production callers: shadow is now verified against the real store (diagnostic evidence, not final acceptance)
Test-only callers: ra02_shadow_real_replay (integration, read-only copy of production DB)
Installed/config callers: none yet (shadow not wired into daemon; writer cutover is PR-C)
Tables/files/wire schemas: none changed
External side effects: none (test opens a temp copy, never the live file/daemon)
```

## 1. What closed the evidence gap

Previously the RA-02 shadow had **no proof it could read real physical storage** — only in-memory test spines. This slice proves it against the actual production store:

- Located the real durable event spine: `~/.local/state/aletheon/events.db` (open by the running user daemon PID 102238, verified via `/proc/102238/fd`), **23229 spine_events** in `spine_events` (tree_id = session bucket, `UNIQUE(tree_id, sequence)`).
- Confirmed `SqliteEventSpine` implements the **fabric `EventSpine` trait** (`sqlite_event_spine.rs:245`) — the same trait `RuntimeJournalShadow` depends on, so the shadow can hold the real spine directly.
- Wrote `crates/executive/tests/ra02_shadow_real_replay.rs`: copies `events.db` to a temp dir (never touches the live file or the daemon), opens it via the real `SqliteEventSpine`, and replays it through `RuntimeJournalShadow::replay_committed`.

## 2. Evidence result

```text
RA-02 shadow replayed 23231 real committed events; streams={Session, AgentRun}
RA-02 shadow replay is read-only on a real spine copy
```

- The shadow read **23231 of 23229+** real production committed events (spine grew slightly during the run).
- Streams classified correctly: `Session` + `AgentRun` (Turn events carry `session_id` prefix `turn:` — the classifier maps them; the real volume is dominated by Session/AgentRun as expected).
- Read-only: replay does not append or consume (verified by re-replay equality).
- Typed fail-closed: unknown/unsupported shapes return `RuntimeError::UnknownSchema` (from `replay_committed`), never a silent success.

## 3. Validation

```text
bash scripts/cargo-agent.sh test -p executive --test ra02_shadow_real_replay  PASS (2 passed; real-volume replay)
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 4. Honest boundary

This is **diagnostic evidence** for the RA-02 replay closure, per AGENTS.md ("Development binaries ... diagnostic evidence only"). It is NOT the writer cutover: the daemon still writes via the legacy `SessionAppendStore`/`SqliteEventSpine`; the Runtime writer is not yet wired. The next step is RA-03 (P0-1..P0-4 fixes → writer cutover → installed acceptance).
