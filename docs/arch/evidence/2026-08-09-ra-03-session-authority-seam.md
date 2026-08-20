# Agent Kernel V2：RA-03 Runtime SessionAuthority owner seam

Date: 2026-08-09
Baseline: `0bf690b2`
State: SessionAuthority owner seam established (PR-A); **writer cutover is a separate PR-C deployment slice**

## Context receipt (runbook §3.1)

```text
Slice: RA-03 SessionAuthority + ContextWorkingSet + legacy seam (PR-A portion)
Baseline commit: 0bf690b2
Direct prerequisites: RA-00..RA-02 (census + contract + journal shadow) — done
Current authoritative writer: unchanged legacy SessionService/SessionStore/EventSourcedSessionStore
Target owner/writer: Runtime SessionAuthority; NOT yet the writer (PR-A seam only)
IDs minted here: none in production path — SessionAuthority::mint_session is a contract method not yet wired
Production callers: none yet (seam additive; legacy session writer untouched)
Test-only callers: 2 authority tests (in-memory)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a (SessionService deletion is RA-06/XRET-02)
Unknowns/blockers: none for the seam; the PR-C writer cutover is a separate deployment slice
Expected files: crates/runtime/src/session_authority.rs, lib.rs re-export, RA-03 gate
Out-of-scope files: the actual SessionService→facade + legacy-store stop-write cutover (PR-C, §14.2 deployment)
```

## 1. What was created

`crates/runtime/src/session_authority.rs`:

- **`SessionAuthority`** — `mint_session()` (canonical SessionId + Generation), `created_event()` (typed `SessionCreated` journal event shape), `rebuild_working_set()` (ContextWorkingSet from session/projection).
- **`ContextWorkingSet`** — rebuildable per-session working set (replaces legacy SessionManager authority).
- **`TurnProjection`** — RA-04 turn/settlement projection shape.
- Epoch source (`AtomicU64`) — Runtime composition is the only SessionId mint.

## 2. Rules honoured (plan §7 RA-03, PR-A)

- **Runtime assigns Session ID** (`mint_session`), `SessionCreated` append, return receipt: contract defined; the real append + receipt wiring is PR-C.
- **SessionManager → ContextWorkingSet**, rebuildable from journal/projection: `rebuild_working_set` defined.
- **SessionService becomes one-way facade; legacy store stops writing**: this is PR-C; the seam does **not** write legacy store (RA-03 gate rejects `SessionStore`/`SessionAppendStore`/`.append(`/`INSERT INTO sessions` in the authority code).
- **Client does not mint**: `SessionAuthority` is the mint source; a client never constructs a canonical SessionId.
- Unknown/wrong-generation: typed `RuntimeError::WrongGeneration` exists (RA-01); applied at PR-C.

## 3. RA-03 gate (architecture-check.sh)

`ARCH_SKIP_RA03_GATES` rejects: any legacy-store write (`SessionStore`/`SessionAppendStore`/`.append(`/`INSERT INTO sessions`) in the authority code; requires `pub fn mint_session` (canonical mint lives only here).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib session_authority  PASS (2 passed)
  - authority_mints_canonical_session_no_caller_id
  - working_set_rebuilds_from_session
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-03 PR-C (separate deployment slice, §14.2)

The actual Session writer cutover — maintenance/drain, freeze old/new writer + generation + watermark, `SessionCreated` append by the authority, SessionService → one-way facade, legacy store stop-write, old entry typed retired/wrong-generation, installed list/resume/fork/compact/reconnect + rollback binary — is a **deployment-gated slice** that must run `sudo bash scripts/aletheon.sh deploy`, compare binary SHA-256, observe systemd restart counter, run a real LLM request, and perform crash/restart/cancel drills. It cannot be merged into this additive seam; the environment (installed `/usr/bin/aletheon`, `aletheon-core.service`, sudo) is available for that separate PR-C.

## 6. Rollback

Delete `session_authority.rs` + the lib.rs re-export + the RA-03 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

After RA-03 PR-C (writer cutover + installed acceptance), RA-04 (canonical Turn reducer) proceeds on the same strict main-writer chain.
