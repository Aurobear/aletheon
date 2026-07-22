# Phase 6 Channel Identity and Information Sources Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Make Executive application and Gateway consume neutral Channel, Identity, Mail, Calendar, File, and ExternalEvent ports while confining Google and Telegram behavior to adapters.

**Architecture:** Build on Phase 1 neutral DTOs and Phase 2 layering. Preserve OAuth/token security, event persistence, channel delivery semantics, and legacy configuration through explicit compatibility boundaries.

**Tech Stack:** Rust 1.85+, Bash, Python 3, Cargo via `scripts/cargo-agent.sh`, repository architecture gates.

---

## Global execution constraints

- Treat `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` as the architecture source of truth.
- Re-read that document and every cited symbol before editing; record changed line anchors in the task report.
- Do not modify files outside the declared paths. Stop if a required change crosses the boundary and report it.
- Preserve unrelated working-tree changes. Never use `git reset --hard`, `git checkout --`, or broad cleanup commands.
- Never invoke Cargo directly. Use `bash scripts/cargo-agent.sh <cargo arguments>` and the narrowest package/test target.
- Do not run concurrent Executive or workspace builds. Only the final integration owner runs workspace-wide commands.
- Keep security-sensitive behavior fail-closed. Do not weaken credential, scope, sandbox, network, lease, or trust checks.
- Each non-trivial commit must use a conventional subject, blank line, problem/solution context, and concrete bullets.
- Before each commit run `git diff --cached --check` and inspect the complete staged diff.
- A task is incomplete if tests pass but its architecture gate, compatibility evidence, or inventory update is missing.

## Prerequisites and owned paths

Prerequisites: Phase 1 + Phase 2 + Phase 0 persistence inventory.

- Gateway domain/ports/application/adapters
- Corpus Google OAuth/API/sync adapters
- Executive external identity, Google sync/store/dispatcher, Gmail channel application paths
- Fabric neutral contracts from Phase 1
- credential/token repositories and security tests

## Task 1: Lock security and delivery behavior

- [x] OAuth state ownership, PKCE, redirect validation, read-only scope allowlist, refresh singleflight, revocation, encrypted token storage, permissions, and fail-closed paths.
- [x] Telegram owner-only filtering, correlation IDs, callbacks, restart recovery, delivery receipts, and health.
- [x] Gmail ingress policy, attachment bounds, deduplication, goal drafting, reporting, and external-event persistence.

## Task 2: Stable capability ports

Use separate ports:

```text
ExternalIdentityProvider
MailSource
CalendarSource
FileSource
ExternalEventSource
ChannelTransport
```

- [x] Ports use neutral IDs/DTOs/errors only.
- [x] Cursor/object IDs remain opaque.
- [x] Application does not depend on a giant GoogleIntegration object.
- [x] Authorization remains an application/security decision; adapter supplies verified identity and capability evidence.

## Task 3: Gateway layering

- [x] Keep intent/effect/notify as domain/application logic.
- [x] Keep ChannelTransport and related contracts in stable facade.
- [x] Move Telegram polling/token/callback types under crate-private adapter.
- [x] Move concrete store under adapters.
- [x] Crate root no longer exports concrete Telegram transport.

## Task 4: Google adapter isolation

- [x] Corpus owns Google wire/OAuth/API conversion.
- [x] Executive adapters own concrete repository/composition where Phase 2 assigns them.
- [x] Executive application sees only neutral ports.
- [x] Provider-specific event matching/formatting does not leak into Goal/channel application paths.
- [x] Credential material never enters shared DTO, event, trace, or Debug output.

## Task 5: Compatibility and persistence

- [x] Old identity/grant/event records from Phase 1 fixtures remain readable.
- [x] Token store encryption and filesystem/database permissions are unchanged.
- [x] Invalid configured identity/channel integration fails closed.
- [x] Optional absent channel/source reports disabled/degraded explicitly, never selects another provider silently.

## Validation

```bash
bash scripts/cargo-agent.sh test -p gateway
bash scripts/cargo-agent.sh test -p corpus --test google_read_only
bash scripts/cargo-agent.sh test -p corpus --test google_delta_sync
bash scripts/cargo-agent.sh test -p executive --test google_tool_flow
bash scripts/cargo-agent.sh test -p executive --test google_event_routing
bash scripts/cargo-agent.sh test -p executive --test google_sync_recovery
bash scripts/cargo-agent.sh test -p executive --test telegram_goal_commands
bash scripts/cargo-agent.sh test -p executive --test telegram_restart_recovery
bash scripts/cargo-agent.sh test -p executive --test gmail_channel_policy
bash tests/architecture_check.sh
```

## Commit stages

1. `test(integrations): lock identity channel and source security`
2. `refactor(gateway): isolate channel transport adapters`
3. `refactor(identity): migrate application to neutral identity ports`
4. `refactor(sources): isolate Google information-source adapters`
5. `chore(arch): enforce channel and identity boundaries`

## Completion evidence (2026-07-23)

- Executive application now exposes `ExternalSourceUseCases`, neutral refresh/error/status names, generic channel/source worker ownership, and contains no Google, Gmail, or Telegram identifier. Concrete identity/source orchestration remains in adapters and host protocol compatibility.
- Gateway keeps dispatch, intent, effects, notification, account selection, and approval logic provider-neutral. Telegram wire DTOs/polling and SQLite storage moved under private `adapters/`; the crate root exposes only a transport factory returning `dyn ChannelTransport` and an opaque stable store facade.
- Approval resolution now uses the inbound channel ID rather than hardcoding a provider. External read preprocessing and event capability names are neutral, including trusted account context markers.
- OAuth/read-only/delta security, event routing/recovery, channel owner/restart behavior, Gmail policy, Gateway tests, and architecture fixtures all pass. Phase 1 persisted identity/event compatibility remains intact.
