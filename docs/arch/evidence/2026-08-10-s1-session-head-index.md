# Aletheon closure plan：S1 EventSourcedSessionStore extensibility

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §15
State: per-session session-head/index seam established; legacy EventSourcedSessionStore stays authoritative

## Context receipt (runbook §3.1)

```text
Slice: S1 EventSourcedSessionStore extensibility (head/index seam)
Baseline commit: 0bf690b2
Plan revision: closure-plan §15
Direct prerequisites: RA-02 (journal) + RA-03 (SessionAuthority) — done
Current authoritative writer: unchanged legacy EventSourcedSessionStore (executive/src/adapters/session/event_sourced_store.rs)
Target owner/writer: Runtime SessionHeadIndex; not yet wired
IDs minted here: none
Production callers: none yet (head/index additive)
Test-only callers: 3 session_head tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam; the append-path cutover + benchmark is PR-C
Expected files: crates/runtime/src/session_head.rs, lib.rs re-export, S1 gate
Out-of-scope files: EventSourcedSessionStore append-path cutover, benchmark
```

## 1. What was created

`crates/runtime/src/session_head.rs`:

- **`SessionHead`** — per-session last sequence + last event id (head only, never full history).
- **`PendingAppend`** — before-commit append; crash recovery distinguishes committed vs unfinished settlement.
- **`SessionHeadIndex`** — per-session sequence allocator + head store + pending tracker.

## 2. Rules honoured (S1)

- **Per-session sequence allocation** from the head (O(1)) — no full-history scan (S1 gate rejects `load_items`/`load_all` in the seam).
- **Per-session lock granularity**: each session has an independent head; different sessions are truly concurrent (no global writer mutex).
- Only last sequence/event id read; not the full item set.
- Snapshot accelerates projection, never replaces event authority (documented).
- Crash recovery identifies committed vs unfinished settlement (`PendingAppend` + `has_unfinished_settlement`).
- (sequence, session) uniqueness via head commit max-keep semantics.

## 3. S1 gate (architecture-check.sh)

`ARCH_SKIP_S1_GATES` requires `SessionHead`/`next_sequence`/`record_pending`/`has_unfinished_settlement`; rejects any full-history load in the seam.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib session_head  PASS (3 passed)
  - sequences_are_per_session_continuous
  - crash_recovery_detects_unfinished_settlement
  - head_reads_last_only_no_full_history
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining S1 (PR-C)

Append-path cutover in `EventSourcedSessionStore` (head-driven sequence + CAS uniqueness), the 1k/10k/100k benchmark with p50/p95/p99, conflict-retry caps, and replay/projection/compaction equivalence — deployment + perf verification.

## 6. Rollback

Delete `session_head.rs` + the lib.rs re-export + the S1 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

C1 (cache correctness), M1 (evidence-driven multi-agent controller), A1 (installation scoreboard) remain in the closure plan.
