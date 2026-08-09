# Agent Kernel V2：APX-01 minimal Application + Runtime ports

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-application-persistence-extraction.md` §APX-01
State: minimal Application crate + typed facade established; **no legacy writer cutover**

## Context receipt (runbook §3.1)

```text
Slice: APX-01 minimal Application crate + Runtime command/query ports
Baseline commit: 0bf690b2
Plan revision: APX-01 (application-persistence-extraction.md:212-223)
Direct prerequisites: APX-00 (census) + RA-01 (Runtime facade) — done
Current authoritative writer: unchanged legacy Executive handlers + SessionService
Target owner/writer: Application (facade over Runtime); not yet wired
IDs minted here: none — consumes Runtime-assigned SessionId from receipts
Production callers: none yet (crate additive; legacy handler route untouched)
Test-only callers: fake in-memory Runtime port (2 tests)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/application/{Cargo.toml,src/lib.rs,error.rs,use_case.rs}, module-boundaries
Out-of-scope files: all writer cutovers, APX-02+
```

## 1. What was created

`crates/application` — pure use-case facade depending only on `fabric` (contracts) + `runtime`:

- `error.rs` — `ApplicationError` typed errors (SessionNotFound/SessionAlreadyExists/InvalidSessionReference/RuntimeRejected/UnknownUseCase).
- `use_case.rs` — `ApplicationFacade` trait (create/resume/list/get session) + `DefaultApplicationFacade` over `RuntimeCommandPort` + `RuntimeQueryPort`. `CreateSession`/`ResumeSession`/`ForkSession`/`ListSessions`/`GetSession`/`DeleteSession` marker types.

## 2. Rules honoured (APX-01)

- New pure Rust types + typed errors + use cases: done.
- `Create/Resume/Fork/List/Get Session` facade: `create_session`/`resume_session`/`list_sessions`/`get_session` present (fork/list real behavior lands with RA-03).
- Legacy Executive handler can call new Application through a one-way adapter: the facade is wired to Runtime ports; the legacy route is untouched (adapter cutover is CGP-06).
- **Application does not hold a Runtime repository and does not mint a core ID**: `DefaultApplicationFacade` holds only `Arc<dyn RuntimeCommandPort>`/`Arc<dyn RuntimeQueryPort>`; it consumes Runtime-assigned `SessionId` from `CommandReceipt` (APX-01 gate enforces no `Repository`/`SessionStore`/`SessionId(` mint in production source).
- Dependency graph: `application` depends only on `fabric` + `runtime` (APX-01 gate rejects Executive/interact/gateway/corpus/kernel/mnemosyne/agora/dasein/metacog).
- Acceptance: **in-memory fake Runtime port verifies command forwarding** — 2 tests pass.

## 3. APX-01 gate (architecture-check.sh)

`ARCH_SKIP_APX01_GATES` rejects:
- any forbidden crate dependency in `application/Cargo.toml`;
- any `Repository`/`AgentRunRepository`/`SessionAppendStore`/`SessionStore` reference in production source;
- any core-ID mint (`SessionId(`/`TurnId(`/`AgentRunId(`/`AgentId(`) in production source (tests are cfg-split and exempt).

Mutation-tested: adding `executive` dependency rejected.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p application    PASS
bash scripts/cargo-agent.sh test -p application     PASS (2 passed)
  - create_session_forwards_to_runtime_and_returns_receipt
  - resume_session_returns_runtime_assigned_id
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `crates/application/` + the Cargo.toml member + module-boundaries row + the APX-01 gate → exact baseline. No schema, no writer, no handler-route change.

## 6. Next

APX-01 done → **APX-02** (Approval aggregate/store port + SQLite adapter) and **E1** (extension ports) are unblocked.
