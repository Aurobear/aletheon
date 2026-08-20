# Agent Kernel V2：RA-03 PR-B Session shadow verifier

Date: 2026-08-10
Baseline: `0bf690b2`
State: read-only Session shadow verifier built; legacy SessionService stays authoritative; writer switch is PR-C

## Context receipt (runbook §3.1)

```text
Slice: RA-03 PR-B Session shadow verifier
Baseline commit: 0bf690b2
Direct prerequisites: RA-03 PR-A (SessionAuthority) + RA-02 (journal shadow) + S1 (session head) — done
Current authoritative writer: unchanged legacy SessionService/SessionStore
Target owner/writer: Runtime SessionAuthority; shadow only (no switch)
IDs minted here: none
Production callers: none yet (shadow additive)
Test-only callers: 2 shadow tests
Installed/config callers: none (not yet deployed as a daemon self-check)
Tables/files/wire schemas: none changed
External side effects: none (shadow never appends/spawns/effects)
Compatibility seam: none
Deletion owner: legacy SessionStore → RA-06/XRET-02
Unknowns/blockers: none for the shadow
Expected files: crates/runtime/src/session_shadow.rs, lib.rs re-export, RA-03B gate
Out-of-scope files: the PR-C writer switch + deployment
```

## 1. What was created

`crates/runtime/src/session_shadow.rs`:

- **`ShadowReport`** — Match{session, last_sequence, streams} / Mismatch{session, reason} / Empty.
- **`SessionShadowVerifier::verify_session`** — replays committed pages through the RA-02 `RuntimeJournalShadow`, classifies streams, compares the legacy last-sequence against the committed max.

## 2. Rules honoured (runbook PR-B)

- **Legacy facade calls the new owner one-way**: the verifier reads through the Runtime journal shadow.
- **Shadow only read/replay/compare, never append/spawn/effect**: RA-03B gate rejects `.append(`/`.spawn(`/`Command::new`/`INSERT INTO` in the shadow; the journal shadow itself forbids append.
- **Typed mismatch, not a silent success**: a legacy sequence beyond the committed max returns `ShadowReport::Mismatch` (test-verified) — the shadow never reports success on unverifiable data.
- Single writer preserved: the shadow does not write; legacy SessionService stays authoritative until PR-C.

## 3. RA-03B gate (architecture-check.sh)

`ARCH_SKIP_RA03B_GATES` rejects any effect call in the shadow; requires `ShadowReport`/`Mismatch`/`replay_committed`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib session_shadow  PASS (2 passed)
  - empty_store_returns_empty_report
  - legacy_beyond_committed_is_a_mismatch_not_a_silent_success
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-03 (PR-C deployment)

Wire the RuntimeSessionWriter (PR-C code, committed) into the daemon with maintenance/drain + freeze + switch, then `sudo bash scripts/aletheon.sh deploy` + installed acceptance (SHA-256 compare, systemd restart counter, real LLM request, crash/restart/cancel drills, rollback binary). This touches ~6 deeply-coupled session_id sites in a running daemon with live sessions, so it must be a careful, independently-deployed PR.

## 6. Rollback

Delete `session_shadow.rs` + the lib.rs re-export + the RA-03B gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

The RA-03 PR-C deployment (writer switch), then RA-04/RA-05 PR-C on the strict main-writer chain.
