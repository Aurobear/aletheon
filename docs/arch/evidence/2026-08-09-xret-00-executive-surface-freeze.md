# Agent Kernel V2：XRET-00 Executive surface freeze & seam ledger

Date: 2026-08-09
Baseline: `0bf690b2`
State: evidence-only surface freeze closed; `executive-surface-ledger.tsv` frozen; no behavior change

## Context receipt

```text
Slice: XRET-00 Executive public surface freeze + seam ledger
Baseline commit: 0bf690b2
Direct prerequisites: RA-00, K0, APX-00, CGP-00, E0, D0
Current authoritative writer: unchanged legacy Executive paths
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: enumerated via executive-layers.tsv + disposition ledger
Test-only callers: excluded
Installed/config callers: cross-checked against retired-authorities.tsv + persistence-surfaces.tsv
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added (COMPAT rows recorded, not created)
Deletion owner: per-row migration_pr / deletion_gate
Unknowns/blockers: INVESTIGATE=47 rows carry per-slice closure before writer cutover
Expected files: config/architecture/executive-surface-ledger.tsv, this evidence record, XRET-00 gate
Out-of-scope files: all production behavior, XRET-01+
```

## 1. Executive surface (executive-surface-ledger.tsv)

372 rows × 7 columns (`path semantics disposition target_owner migration_pr deletion_gate blocker`), generated from `2026-08-08-executive-source-disposition-ledger.md` §2:

| disposition | count | note |
|---|---:|---|
| SPLIT | 156 | split across owners |
| MERGE | 123 | merge into owner |
| INVESTIGATE | 47 | must converge to final disposition before its writer cutover |
| DELETE | 25 | delete with caller evidence |
| MOVE | 19 | move to owner |
| COMPAT | 2 | `legacy_session_service.rs`, `core/sub_agent.rs` — one-way seams |

5 rows are marked `RETIRED@PR192` (already deleted by foundation: `application/agent/**`, `composition/agents/**`, `core/session.rs`).

## 2. COMPAT cardinality (plan §5 XRET-00)

The two `COMPAT` exact paths each map to exactly one seam row in the disposition ledger, and the two seam rows map back to exactly one COMPAT path each — bidirectional cardinality = 1:

- `crates/aletheon/src/wiring/daemon/legacy_session.rs` → `runtime/compat` (RA-00..RA-06, B1+B7)
- `crates/executive/src/core/sub_agent.rs` → `runtime/compat` (RA-00..RA-06, B1+B7)

No other COMPAT row exists; the gate enforces this stays exactly 2.

## 3. Monotonic gates added

- **No new Executive module/public re-export/concrete adapter**: the 367-file Executive source set is frozen; a new file under `crates/executive/src` fails until the ledger and `executive-layers.tsv` are updated together.
- **Ledger ↔ source consistency**: every ledger path must exist (except `RETIRED@PR192`), and every Executive source file must have a ledger row (372 = 367 live + 5 retired).
- **COMPAT cardinality = 2**; disposition vocab restricted to {SPLIT,MERGE,MOVE,INVESTIGATE,DELETE,COMPAT}.
- Legacy route/read observability counters remain in place (no secret/payload) via the existing CGP-00 route census; XRET-00 adds the surface freeze, not new payload telemetry.

## 4. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p executive --lib  PASS
```

## 5. Scope boundaries

- **Allowed**: executive-surface-ledger.tsv, this evidence record, XRET-00 gate.
- **Explicitly not done**: no `XRET-01` facade cut, no module deletion, no concrete adapter removal, no `executive-layers.tsv` schema change, no behavior change.
