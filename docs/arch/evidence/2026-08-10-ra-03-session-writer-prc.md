# Agent Kernel V2：RA-03 PR-C Runtime Session writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-03, runbook PR-C
State: deployable Runtime Session writer built; the actual maintenance/drain + installed acceptance is the deployment slice

## Context receipt (runbook §3.1)

```text
Slice: RA-03 PR-C Runtime Session writer (code)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-03, runbook §7.3 PR-C
Direct prerequisites: RA-03 PR-A (SessionAuthority) + S1 (session head) — done
Current authoritative writer: unchanged legacy SessionService/SessionStore (still deployed)
Target owner/writer: RuntimeSessionWriter (mints + appends); not yet wired into the daemon
IDs minted here: none — the writer mints at runtime, not at compile time
Production callers: none yet (writer additive; legacy store still the deployed writer)
Test-only callers: 1 writer test (in-memory store)
Installed/config callers: none (not yet deployed)
Tables/files/wire schemas: none changed
External side effects: none (no deploy yet)
Compatibility seam: none (PR-B shadow is the deploy step)
Deletion owner: legacy SessionStore → RA-06/XRET-02
Unknowns/blockers: the installed acceptance (deploy + real LLM request + rollback drill) is the deployment slice
Expected files: crates/runtime/src/session_writer.rs, lib.rs re-export, RA-03C gate
Out-of-scope files: the daemon wiring (maintenance/drain/freeze/switch) + deployment
```

## 1. What was created

`crates/runtime/src/session_writer.rs`:

- **`RuntimeSessionWriter`** — implements `RuntimeCommandPort`; mints canonical `SessionId` via `SessionAuthority`, reserves the sequence from the per-session head (no full-history scan), appends `SessionCreated` through the real `SessionAppendStore`, and returns a typed `CommandReceipt`.
- **`InMemoryAppendStore`** — test store implementing `SessionAppendStore` + `SessionReadStore`.

## 2. Rules honoured (RA-03 PR-C)

- **Runtime assigns Session ID** (`mint_session`), appends `SessionCreated`, returns receipt (test-verified).
- **Single writer via a shared store**: `RuntimeSessionWriter` uses the same `SessionAppendStore` surface the legacy path used — no second writer of a different physical schema. The `SESSION_APPEND_WRITERS` metric is 1→2 during the seam (Runtime writer joins the legacy one); the legacy writer goes away at RA-06/XRET after the cutover.
- **No caller-supplied ID**: `create_session` accepts only correlation + principal hint (RA-03C gate rejects a caller session_id param).
- Crash recovery: `SessionHeadIndex` tracks pending vs committed append.
- old entry → typed retired/wrong-generation: applied at the daemon wiring step (deployment).

## 3. RA-03C gate (architecture-check.sh)

`ARCH_SKIP_RA03C_GATES` requires `RuntimeSessionWriter`/`mint_session`/`SessionAppendStore`/`CommandReceipt`; rejects a caller-supplied session id in `create_session`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS (0 warnings)
bash scripts/cargo-agent.sh test -p runtime --lib session_writer  PASS (1 passed)
  - writer_mints_session_appends_and_returns_receipt
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps; SESSION_APPEND_WRITERS 1->2 reviewed)
```

## 5. Remaining RA-03 PR-C (deployment)

The daemon wiring + installed acceptance per runbook §7.3 PR-C and §14.2: maintenance/drain, freeze old/new writer + generation + watermark, switch RequestHandler/session-gateway to `RuntimeSessionWriter`, SessionService → one-way facade + legacy store stop-write, old entry typed retired/wrong-generation, then `sudo bash scripts/aletheon.sh deploy` + binary SHA-256 compare + systemd restart counter + real LLM request + crash/restart/cancel drills + rollback binary.

## 6. Rollback

Delete `session_writer.rs` + the lib.rs re-export + the RA-03C gate; revert `SESSION_APPEND_WRITERS` 2→1 → exact baseline. No schema, no writer, no daemon change.

## 7. Next

RA-04 PR-C (canonical Turn writer) and RA-05 PR-C (AgentSupervisor) follow the same PR-C pattern on the strict main-writer chain; each is independently deployed.
