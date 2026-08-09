# Agent Kernel V2：RA-02 RuntimeJournal read-only shadow

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §12.1
State: RuntimeJournal shadow established; **no writer cutover, no append/spawn/effect**

## Context receipt (runbook §3.1)

```text
Slice: RA-02 durable RuntimeJournal shadow
Baseline commit: 0bf690b2
Plan revision: implementation-plan §12.1
Direct prerequisites: RA-01 (Runtime contract) + K0 (EventSpine census) — done
Current authoritative writer: unchanged legacy SessionAppendStore/EventSpine/AgentRun writers
Target owner/writer: Runtime (canonical journal); not yet the writer
IDs minted here: none
Production callers: none yet (shadow additive; legacy writers untouched)
Test-only callers: 1 shadow test (empty spine read-only replay)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/runtime/src/journal.rs, lib.rs re-export, Cargo.toml (fabric dep), RA-02 gate
Out-of-scope files: all writer cutovers, RA-03+
```

## 1. What was created (plan §12.1)

`crates/runtime/src/journal.rs` — read-only shadow journal over the existing `fabric::EventSpine`:

- **`StreamKind`**: Session / Turn / AgentRun — separated streams.
- **`ShadowEntry`**: `stream`, `position`, `schema_version`, `digest` (never the payload — no secret material).
- **`ShadowMismatch`**: typed UnknownStream / UnsupportedVersion / OutOfOrder.
- **`RuntimeJournalShadow`**: `new(spine)` + `replay_committed(after, through, limit)` — **bounded replay only**.
- Classifier: agent_id present → AgentRun; `turn:` session prefix → Turn; `session` prefix → Session; otherwise version 0 (shadow comparisons fail closed on unknown).

## 2. Rules honoured (plan §12.1)

| rule | status |
|---|---|
| adapt existing Session append/EventSpine/AgentRun physical schema | shadow reads `EventSpine` committed pages (fabric dep added to runtime) |
| separate Agent/Session/Turn streams, sequence, schema, generation, digest | `StreamKind` + `ShadowEntry` (sequence=position, schema_version, digest) |
| bounded replay + typed mismatch | `replay_committed` is page-bounded; `ShadowMismatch` typed |
| **shadow forbids append/spawn/effect** | RA-02 gate rejects `.append(`/`.spawn(`/`Command::new`/fs-write/socket; the shadow only reads |
| unknown event/version fail closed | classifier returns version 0 for unknown shapes → comparisons fail closed |
| no irreversible migration | none performed |

## 3. RA-02 gate (architecture-check.sh)

`ARCH_SKIP_RA02_GATES` rejects (Rust code only, comments exempt): any `.append(`/`.spawn(`/`Command::new`/`std::fs::write`/`UnixStream`/`UnixListener`/`reqwest`; requires `replay_committed` and forbids a `pub fn append`/`async fn append`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib journal  PASS (1 passed)
  - shadow_replay_is_read_only_and_returns_empty_page
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `journal.rs` + the lib.rs re-export + the fabric/anyhow Cargo deps (if unused) + the RA-02 gate → exact baseline. No schema, no writer, no table change.

## 6. Next

RA-02 done → **RA-03 Session writer cutover** (the first writer switch: maintenance/drain, Runtime-assigned SessionId, `SessionCreated` append, legacy store stop-write). This is a deployment-gated slice per §14.2 (official socket + rollback binary verification).
