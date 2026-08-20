# Agent Kernel V2：RA-03 PR-C Runtime Session writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
State: Runtime Session writer is wired into daemon composition; installed equivalence and rollback evidence remain open

## Context receipt (runbook §3.1)

```text
Slice: RA-03 PR-C Runtime Session writer (code)
Baseline commit: 0bf690b2
Direct prerequisites: RA-03 PR-A (SessionAuthority) + S1 (session head) — done
Current authoritative writer: RuntimeSessionWriter in `[bootstrap].session_writer = "runtime"`, behind the SessionService facade
Target owner/writer: RuntimeSessionWriter (mints + appends) over canonical `sessions-v1.db`/shared EventSpine
IDs minted here: RuntimeSessionWriter mints the canonical SessionId at daemon bootstrap and for subsequent creates
Production callers: `open_daemon_session` and `build_turn_services` consume the single `SessionInfrastructure` instance
Test-only callers: 1 writer test (in-memory store)
Installed/config callers: `config/default.toml:8-17` selects Runtime mode; legacy `sessions.db` is migration/explicit rollback input only
Tables/files/wire schemas: canonical `sessions-v1.db` and shared `events.db` are opened once and reconciled before admission
External side effects: Runtime mode migrates legacy rows, appends `SessionCreated`, and projects through the canonical store
Compatibility seam: legacy read/migration and explicit rollback remain; production writes use the Runtime writer
Deletion owner: legacy SessionStore → RA-06/XRET-02
Unknowns/blockers: maintenance/drain watermark, session operation equivalence, failure matrix, and rollback drill
Expected files: runtime writer, SessionInfrastructure, daemon service injection, config, and RA-03C gate
Out-of-scope files: RA-04/RA-05 writer semantics and RA-06 physical deletion
```

## 1. What was created

`crates/runtime/src/session_writer.rs`:

- **`RuntimeSessionWriter`** — implements `RuntimeCommandPort`; mints canonical `SessionId` via `SessionAuthority`, reserves the sequence from the per-session head (no full-history scan), appends `SessionCreated` through the real `SessionAppendStore`, and returns a typed `CommandReceipt`.
- **`InMemoryAppendStore`** — test store implementing `SessionAppendStore` + `SessionReadStore`.

## 2. Rules honoured (RA-03 PR-C)

- **Runtime assigns Session ID** (`mint_session`), appends `SessionCreated`, returns receipt (test-verified).
- **Single writer via a shared store**: `RuntimeSessionWriter` uses the injected `SessionAppendStore` over canonical `sessions-v1.db`; the static `SESSION_APPEND_WRITERS` census still counts retained compatibility definitions, not two active production writers.
- **No caller-supplied ID**: `create_session` accepts only correlation + principal hint (RA-03C gate rejects a caller session_id param).
- Crash recovery: `SessionHeadIndex` tracks pending vs committed append.
- old entry → typed retired/wrong-generation: applied at the daemon wiring step (deployment).

## 3. RA-03C gate (architecture-check.sh)

`ARCH_SKIP_RA03C_GATES` requires `RuntimeSessionWriter`/`mint_session`/`SessionAppendStore`/`CommandReceipt`; rejects a caller-supplied session id in `create_session`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS (0 warnings)
bash scripts/cargo-agent.sh test -p runtime --lib  PASS (55 passed)
  - session writer, event-spine, generation, and terminal-fence coverage
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps; retained compatibility definitions reviewed)
```

## 5. Remaining RA-03 PR-C (acceptance and deletion gate)

The daemon wiring is complete. Remaining evidence per runbook §7.3 PR-C and §14.2 is the maintenance/drain/freeze watermark record, installed list/resume/fork/compact/reconnect equivalence, crash/restart/cancel drills, and rollback-binary drill. After those pass, perform the RA-06 caller-zero/deletion inventory; do not reintroduce a legacy production writer.

## 6. Rollback

Delete `session_writer.rs` + the lib.rs re-export + the RA-03C gate; revert `SESSION_APPEND_WRITERS` 2→1 → exact baseline. No schema, no writer, no daemon change.

## 7. Next

RA-04 PR-C (canonical Turn writer) and RA-05 PR-C (AgentSupervisor) follow the same PR-C pattern on the strict main-writer chain; each is independently deployed.
