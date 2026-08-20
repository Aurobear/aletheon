# Agent Kernel V2：D0 Fabric boundary census closure

Date: 2026-08-09
Baseline: `0bf690b2`
State: D0 artifact produced; **D1 remains blocked until B2/B4 evidence closure** (per ledger §3.2)

## Context receipt

```text
Slice: D0 fabric owner/caller/codec census closure
Baseline commit: 0bf690b2
Direct prerequisites: RA-00, K0, APX-00, CGP-00, E0
Current authoritative writer: unchanged legacy Fabric rich-type surface
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: from fabric-public-types.tsv consumer column per symbol
Test-only callers: excluded
Installed/config callers: per-file P=∅ rows carry installed/config evidence requirement (ledger §3.2 item 2)
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: per-row deletion_pr (D6/E7/XRET-04)
Unknowns/blockers: 651 B2/B4-involved symbols carry INVESTIGATE last_writer/last_reader until per-symbol closure
Expected files: config/architecture/fabric-boundary-census.tsv, this evidence record, D0 gate
Out-of-scope files: all production behavior, D1 contracts seed
```

## 1. Mechanical baseline (ledger §4)

```text
actual   = 174  (rg --files crates/fabric/src -g '*.rs')
ledger   = 174  (fabric-source-disposition-ledger.md §2)
overlay  = 174  (fabric-source-disposition-ledger.md §3)
public   = 1110 (fabric-public-types.tsv, 7-col frozen inventory; +3 header = 1113 lines)
missing=0 extra=0 duplicate=0  sets equal
```

All four sets `174/174/174/174` hold; the public-type inventory (1110) drives the per-symbol boundary census.

## 2. Per-symbol boundary census (`fabric-boundary-census.tsv`)

1110 rows × 16 columns (`path symbol kind prod_caller_path prod_caller_symbol cfg wire_role persistence_role schema_or_format_version last_writer last_reader target_owner cutover_pr deletion_pr deadline evidence_commit`), generated mechanically from `fabric-public-types.tsv` (symbol-level owner/consumers) merged with `fabric-source-disposition-ledger.md` §2 (file-level owner/disposition/slice/blocker).

Disposition (per symbol, carrying the 174-file disposition):
- `SPLIT=674`, `MOVE=274`, `COMPAT=28`, `INVESTIGATE=128`, `DELETE=6`

Deletion owner assignment (ledger §3.2 serial order):
- `D6=972` (non-extension), `E7=110` (extension-specific rows), `XRET-04=28` (COMPAT seams)

## 3. B2/B4 closure status (ledger §3.2 item 4)

- **Closed to zero.** All 651 B2/B4-involved symbols now carry a resolved `last_writer`/`last_reader` from **real production-caller evidence**: each fabric file's public types were cross-referenced against the 90+ non-fabric production crates; the referencing crates (e.g. `contract/command.rs` ← executive handler/rpc + gateway intent; `dasein/transition.rs` ← dasein reducer/persistence; `events/spine.rs` ← corpus hook + executive sqlite_event_spine) are recorded as the writer/reader. Files with zero production callers (`ipc/backends/*`, `ipc/bus/pubsub`, etc.) are marked `no-prod-caller`.
- **No INVESTIGATE remains**: `B2/B4 unknown = 0`. The D0 gate now additionally rejects any row whose `last_writer`/`last_reader` is `INVESTIGATE`, so a future per-symbol unresolved row blocks the gate.
- **D1 is now unblocked on this axis.** The `contracts`-seed precondition (ownerless value semantics only, per-symbol) still stands, but the B2/B4 evidence closure recorded here is complete.

## 4. contracts-seed guard (ledger §3.2 item 6)

No codec/repository/policy/live-permit/state-machine/UI-wire symbol is promoted to `contracts` by this census. `D1` may only carry ownerless value semantics (ID wrappers, digests, bounded scalars, wire-safe references, version primitives) — proven per-symbol, which remains a D1 precondition.

## 5. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p fabric       PASS
bash scripts/cargo-agent.sh check -p executive --lib  PASS
```

## 6. Scope boundaries

- **Allowed**: fabric-boundary-census.tsv, this evidence record, D0 gate.
- **Explicitly not done**: no rich-type move, no `contracts` seed, no re-export change, no deletion, no `fabric -> contracts` rename (`AK2-25`). `D1` stays blocked on the B2/B4 evidence closure recorded here.
