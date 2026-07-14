# Aletheon Personal Agent Integration Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a restart-safe Telegram-to-Goal-to-worker/Pi-to-verification-to-approval vertical slice, then add secure Google and optional GBrain integrations without inventing capabilities the current code lacks.

**Architecture:** Durable domain state lives in SQLite repositories; live work uses the existing `TurnPipeline`, `ProcessTable`, `OperationTable`, admission, approval, and memory interfaces. Runtime choice/retry is per attempt, verification is a cancellable job stage, Google credentials are encrypted before persistence, and GBrain composes behind `MemoryService`.

**Tech Stack:** Rust, Tokio, SQLite/rusqlite, serde, existing Aletheon kernel/executive/corpus/mnemosyne abstractions, Telegram Bot API, OAuth 2.0, MCP, reqwest, git worktrees.

---

## 1. How to execute this program

This is a master plan for several independently releasable subsystems. Do not implement all milestones in one branch. Each milestone must receive its own detailed TDD execution plan immediately before implementation; the tasks below define exact scope, contracts, files, validation, and completion gates for that plan.

Dependency order:

```text
M0 -> M1 -> M2 -> M3 -> M4 -> M5
       |      |                  |
       |      +-------> M8       +---- deployment hardening
       +-----> M6 -> M7
```

Every milestone ends with `cargo fmt --all -- --check`, scoped tests, `cargo test --workspace`, and `cargo build --workspace`. Do not begin a dependent milestone while its prerequisite has failing validation.

## 2. M0 — Baseline and schema conventions

**Requirement anchors:** preserve existing conversation behavior (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:52-59`) and use persistent Goals (`docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:6-44`).

**Code anchors:** daemon execution is `TurnPipeline` (`crates/executive/src/service/turn_pipeline.rs:40-67`); generic process state is `crates/fabric/src/types/process.rs:60-90`.

### Task M0.1: Record baseline behavior

**Files:**
- Create: `crates/executive/tests/agent_integration_baseline.rs`
- Inspect: `crates/executive/tests/turn_service_equivalence.rs`
- Inspect: `crates/executive/tests/turn_pipeline_order.rs`

- [ ] Add a regression test proving an ordinary chat turn still completes through the existing path without Goal metadata.
- [ ] Add a regression test proving current `ProcessState` transitions remain unchanged.
- [ ] Run `cargo test -p executive --test agent_integration_baseline`; expect PASS.
- [ ] Run `cargo test -p executive --test turn_service_equivalence --test turn_pipeline_order`; expect PASS.

### Task M0.2: Define SQLite migration conventions

**Files:**
- Create: `crates/executive/src/impl/persistence/mod.rs`
- Create: `crates/executive/src/impl/persistence/migrations.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Test: unit tests beside `migrations.rs`

- [ ] Write failing tests for fresh-database migration, repeated migration, transaction rollback, and schema-version rejection.
- [ ] Implement numbered, transactional migrations using the repository's existing SQLite dependency and connection conventions.
- [ ] Require unique constraints for provider IDs and optimistic `version` columns for mutable aggregate rows.
- [ ] Run `cargo test -p executive -- impl::persistence::migrations`; expect PASS.

### Task M0.3: Baseline gate

- [ ] Run `cargo fmt --all -- --check`; expect exit 0.
- [ ] Run `cargo test --workspace`; expect exit 0.
- [ ] Run `cargo build --workspace`; expect exit 0.
- [ ] Inspect the staged diff and commit only M0 files with a conventional subject and a body explaining the baseline and migration guarantees.

**M0 acceptance:** existing turn/process behavior is protected and later repositories share one migration discipline.

## 3. M1 — Durable Telegram chat vertical slice

**Requirement anchors:** Telegram owner binding, long polling, mapping, offset persistence, and unknown-user rejection (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:61-70`).

### Task M1.1: Define channel contracts

**Files:**
- Create: `crates/fabric/src/types/channel.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Test: unit tests beside `channel.rs`

- [ ] Define serializable `ChannelId`, `MessageId`, `ConversationId`, `ExternalSenderId`, `InboundMessage`, `OutboundMessage`, `MessageContent`, `UserAction`, `ActionType`, and `ChannelHealth`.
- [ ] Keep provider identities external; do not claim they are already `PrincipalId` before binding.
- [ ] Write round-trip tests for text, command, and approval-action messages.
- [ ] Run `cargo test -p fabric -- types::channel`; expect PASS.

### Task M1.2: Add durable inbox and binding repositories

**Files:**
- Create: `crates/executive/src/impl/channel/mod.rs`
- Create: `crates/executive/src/impl/channel/inbox.rs`
- Create: `crates/executive/src/impl/channel/binding.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Modify: `crates/executive/src/impl/persistence/migrations.rs`

- [ ] Add `channel_inbox`, `channel_outbox`, `channel_cursor`, and `channel_binding` tables.
- [ ] Enforce unique `(channel_id, message_id)` inbox rows and unique external binding keys.
- [ ] Write failing tests for duplicate insert, incomplete-row recovery, cursor advancement, unknown binding, and owner pre-binding.
- [ ] Implement repository transactions so result/outbox persistence, inbox completion, and cursor advancement commit atomically.
- [ ] Run `cargo test -p executive -- impl::channel::inbox impl::channel::binding`; expect PASS.

### Task M1.3: Implement router-to-TurnPipeline adapter

**Files:**
- Create: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/channel_router.rs`

- [ ] Write tests proving unknown senders are rejected before an LLM call, duplicate messages create one turn, one conversation is ordered, and two conversations may progress independently.
- [ ] Route chat input into the existing daemon `TurnPipeline`; do not introduce a `SessionService` type.
- [ ] Persist outbound replies before marking the inbound row complete.
- [ ] Add cancellation-aware shutdown that stops intake, drains bounded work, and leaves incomplete rows recoverable.
- [ ] Run `cargo test -p executive --test channel_router`; expect PASS.

### Task M1.4: Implement Telegram provider

**Files:**
- Create: `crates/executive/src/impl/channel/telegram/mod.rs`
- Create: `crates/executive/src/impl/channel/telegram/polling.rs`
- Create: `crates/executive/src/impl/channel/telegram/formatting.rs`
- Modify: `crates/executive/Cargo.toml`
- Modify: daemon configuration files discovered during the milestone plan

- [ ] Add mocked-provider tests for owner allow-list, unknown-user rejection, stable message IDs, offset restart, retry/backoff, button callbacks, and reply formatting.
- [ ] Implement long polling with persisted cursor and bounded exponential backoff.
- [ ] Support `/start` and `/chat`; parse Goal commands but return a clear “Goal runtime not enabled” response until M2.
- [ ] Never advance the Telegram offset before the local inbox transaction commits.
- [ ] Run `cargo test -p executive -- impl::channel::telegram`; expect PASS.

### Task M1.5: M1 release gate

- [ ] Execute a mocked end-to-end owner chat and daemon restart test.
- [ ] Run full workspace formatting, tests, and build.
- [ ] Commit M1 in reviewable stages: contracts, persistence, router, Telegram adapter, validation.

**M1 acceptance:** owner phone chat works; duplicate or replayed updates do not duplicate turns; unknown users consume no model resources; restart resumes incomplete work.

## 4. M2 — One persistent bounded Goal

**Requirement anchors:** persistent Goal and immutable intent (`docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:6-44`, `:81`); bounded tick (`:83-125`); pause/resume/cancel (`:343-369`).

### Task M2.1: Define Goal domain types without changing ProcessState

**Files:**
- Create: `crates/fabric/src/types/goal.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Define `GoalId`, `GoalSpec`, `GoalState`, `GoalWaitReason`, `GoalBudget`, `GoalBudgetUsage`, `GoalVersion`, `GoalSnapshot`, and typed transition errors.
- [ ] Keep Goal states out of `ProcessState`; link a Goal to `Option<ProcessId>` only while live.
- [ ] Write tests for the legal transition matrix, immutable original intent, serde compatibility, and terminal states.
- [ ] Run `cargo test -p fabric -- types::goal`; expect PASS.

### Task M2.2: Implement GoalRepository

**Files:**
- Create: `crates/executive/src/impl/goal/mod.rs`
- Create: `crates/executive/src/impl/goal/repository.rs`
- Create: `crates/executive/src/impl/goal/model.rs`
- Modify: `crates/executive/src/impl/persistence/migrations.rs`

- [ ] Add `goals`, `goal_events`, `goal_tasks`, `goal_attempts`, and `goal_budget_ledger` tables.
- [ ] Write tests for create/load/list, optimistic-version conflict, transition+journal atomicity, immutable spec fields, and non-terminal recovery.
- [ ] Store timestamps and versions explicitly; do not serialize the entire aggregate as an opaque unqueryable blob.
- [ ] Run `cargo test -p executive -- impl::goal::repository`; expect PASS.

### Task M2.3: Implement bounded GoalCoordinator

**Files:**
- Create: `crates/executive/src/impl/goal/coordinator.rs`
- Create: `crates/executive/src/impl/goal/budget.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/goal_lifecycle.rs`

- [ ] Write tests proving one tick performs at most one transition/attempt, budget exhaustion blocks before work, deadline expiry fails predictably, pause prevents execution, cancellation reaches `OperationTable`, and restart clears stale process linkage.
- [ ] Create a kernel process when a Goal begins/resumes; retain Goal state in SQLite when that process exits.
- [ ] Reserve and settle token/cost/attempt usage separately from capability admission.
- [ ] Run `cargo test -p executive --test goal_lifecycle`; expect PASS.

### Task M2.4: Wire Telegram Goal commands

**Files:**
- Modify: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/impl/channel/telegram/formatting.rs`
- Test: `crates/executive/tests/telegram_goal_commands.rs`

- [ ] Test `/goal`, `/goals`, `/status`, `/pause`, `/resume`, and `/cancel` against a temporary database.
- [ ] Compile the user's intent into a Draft `GoalSpec`, show it to the owner, and require approval before entering Ready.
- [ ] Ensure `/status` reads the repository rather than an in-memory `SubAgentSpawner` map.
- [ ] Run `cargo test -p executive --test telegram_goal_commands`; expect PASS.

### Task M2.5: M2 release gate

- [ ] Create a Goal, stop the daemon, restart it, and prove original intent/state/budget remain intact.
- [ ] Run full workspace formatting, tests, and build.
- [ ] Commit domain types, repository, coordinator, command wiring, and validation separately.

**M2 acceptance:** one active Goal survives restart, performs bounded ticks, and supports status/pause/resume/cancel without modifying generic `ProcessState`.

## 5. M3 — Per-attempt runtime, retry, and escalation

**Requirement anchors:** independent attempts (`docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:166-181`), bounded retry/escalation (`:183-237`), model roles (`:270-296`).

### Task M3.1: Add per-spawn runtime selection

**Files:**
- Modify: `crates/executive/src/core/sub_agent.rs`
- Create: `crates/executive/src/core/runtime_registry.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Test: existing `crates/executive/tests/supervision.rs` plus new runtime-selection cases

- [ ] Preserve `with_runtime()` as the default compatibility path.
- [ ] Add `RuntimeId`, `RuntimeRegistry`, and a per-spawn method selecting a registered runtime.
- [ ] Write tests proving two simultaneous agents use different runtimes and a missing runtime fails before process execution.
- [ ] Do not add provider selection behavior to `SupervisorTree`.
- [ ] Run `cargo test -p executive --test supervision`; expect PASS.

### Task M3.2: Implement DeepSeek runtime as a normal provider-backed worker

**Files:**
- Create: `crates/executive/src/impl/runtime/mod.rs`
- Create: `crates/executive/src/impl/runtime/deepseek.rs`
- Modify: `crates/executive/src/impl/mod.rs`

- [ ] Test prompt construction, cancellation, usage extraction, tool-call limits, and provider errors with a fake `LlmProvider`.
- [ ] Call `LlmProvider::complete()` as defined at `crates/fabric/src/types/llm_types.rs:61-65`.
- [ ] Keep retry outside the runtime; one `run()` call is one attempt.
- [ ] Run `cargo test -p executive -- impl::runtime::deepseek`; expect PASS.

### Task M3.3: Add AttemptCoordinator

**Files:**
- Create: `crates/executive/src/impl/goal/attempt.rs`
- Create: `crates/executive/src/impl/goal/retry.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`

- [ ] Define structured `FailureClass`, `RetryPolicy`, `EscalationStep`, and `AttemptEvidence`.
- [ ] Test transient backoff, compile-evidence retry, non-retryable auth/policy failure, repeated-failure escalation, budget exhaustion, and cancellation during backoff.
- [ ] Persist every attempt before scheduling the next one.
- [ ] Use `SupervisorTree` only for unexpected live-process restart limits.
- [ ] Run `cargo test -p executive -- impl::goal::attempt impl::goal::retry`; expect PASS.

### Task M3.4: M3 release gate

- [ ] Demonstrate a failed worker attempt retrying with evidence and escalating to a distinct fake runtime.
- [ ] Run full workspace validation.
- [ ] Commit runtime registry, DeepSeek runtime, attempt policy, and integration separately.

**M3 acceptance:** runtime selection is per agent; attempts are durable; retry/backoff/model switching are explicit and bounded.

## 6. M4 — Pi worktree and verification

**Requirement anchors:** Pi isolation (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:94-105`) and verification evidence (`:107-116`).

### Task M4.1: Define coding job contracts

**Files:**
- Create: `crates/fabric/src/types/coding_job.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Define `CodingJobSpec`, `WorkspaceBoundary`, `ChangedFile`, `CodingJobReport`, `VerificationCheck`, and `VerificationReport`.
- [ ] Test serde and path-boundary normalization, including traversal and symlink escape cases.
- [ ] Run `cargo test -p fabric -- types::coding_job`; expect PASS.

### Task M4.2: Implement WorktreeManager

**Files:**
- Create: `crates/corpus/src/tools/subagent/mod.rs`
- Create: `crates/corpus/src/tools/subagent/worktree.rs`
- Modify: `crates/corpus/src/tools/mod.rs`

- [ ] Test creation from a known base commit, main-worktree immutability, diff collection, success cleanup, failed-worktree retention, TTL pruning, count cap, and disk-budget refusal.
- [ ] Invoke git through `tokio::process::Command` with cancellation and timeouts.
- [ ] Never run destructive cleanup outside the configured worktree base.
- [ ] Run `cargo test -p corpus -- tools::subagent::worktree`; expect PASS.

### Task M4.3: Implement PiRuntime with fail-closed sandboxing

**Files:**
- Create: `crates/executive/src/impl/runtime/pi.rs`
- Modify: `crates/executive/src/impl/runtime/mod.rs`
- Test: `crates/executive/tests/pi_runtime.rs`

- [ ] Test unavailable sandbox, network denial configuration, timeout, cancellation, stdout/stderr capture, process-group termination, forbidden-path modification, and report generation.
- [ ] Treat no isolation as an error; do not silently use a no-op backend.
- [ ] Make one runtime call correspond to one durable attempt.
- [ ] Run `cargo test -p executive --test pi_runtime`; expect PASS.

### Task M4.4: Implement VerificationService

**Files:**
- Create: `crates/executive/src/service/verification/mod.rs`
- Create: `crates/executive/src/service/verification/command.rs`
- Create: `crates/executive/src/service/verification/policy.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Test: `crates/executive/tests/verification_service.rs`

- [ ] Test pass/fail/timeout/cancel/output-limit behavior for format, check/build, relevant tests, diff scope, and capability policy.
- [ ] Run commands with `tokio::process::Command`; never block an async worker with `std::process::Command`.
- [ ] Mark required checks blocking and clippy/architecture advisory for the initial release.
- [ ] Persist the verification report before moving the Goal to AwaitingApproval.
- [ ] Run `cargo test -p executive --test verification_service`; expect PASS.

### Task M4.5: M4 release gate

- [ ] Execute a fake coding change in a temporary repository and prove the main worktree is unchanged.
- [ ] Demonstrate that failed required verification prevents completion.
- [ ] Run full workspace validation and commit in focused stages.

**M4 acceptance:** Pi edits only an isolated worktree, produces a structured report, and cannot reach approval without required evidence.

## 7. M5 — Durable approval and controlled apply

**Code anchors:** current synchronous approval gate is `crates/corpus/src/security/socket_approval.rs:15-54`; current pump is `crates/executive/src/service/turn_pipeline.rs:604-633`.

### Task M5.1: Add durable ApprovalRepository

**Files:**
- Create: `crates/executive/src/impl/approval/mod.rs`
- Create: `crates/executive/src/impl/approval/repository.rs`
- Modify: `crates/executive/src/impl/persistence/migrations.rs`

- [ ] Define request ID, Goal/attempt references, operation category, risk, summary, expiry, status, and resolution metadata.
- [ ] Test one-time resolution, duplicate callbacks, expiry-to-deny, restart recovery, wrong-principal denial, and optimistic conflict.
- [ ] Keep `SocketApprovalGate` for synchronous live-turn tool requests; do not overload it as the durable store.
- [ ] Run `cargo test -p executive -- impl::approval`; expect PASS.

### Task M5.2: Route trusted Telegram approvals

**Files:**
- Modify: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/impl/channel/telegram/formatting.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`

- [ ] Test approve/reject buttons, textual commands, expired IDs, replay, and untrusted sender.
- [ ] Bind every approval to one owner principal and one immutable operation summary.
- [ ] Resume the Goal by repository transition after approval; never rely on an in-memory oneshot surviving restart.
- [ ] Run approval and Telegram command tests; expect PASS.

### Task M5.3: Add controlled apply operation

**Files:**
- Create: `crates/corpus/src/tools/subagent/apply.rs`
- Modify: `crates/corpus/src/tools/subagent/mod.rs`
- Modify: Goal coordinator integration files

- [ ] Test stale base commit, conflicting patch, scope escape, rejected approval, cancellation, and successful apply.
- [ ] Require a valid unexpired approval and re-check diff scope immediately before applying.
- [ ] Do not push, merge, or mutate protected branches automatically.
- [ ] Run scoped apply tests; expect PASS.

### Task M5.4: First usable release gate

- [ ] Run the complete mocked Telegram `/goal` flow through restart, worker, Pi, verification, approval, controlled apply, and completion summary.
- [ ] Prove rejection leaves the main worktree unchanged.
- [ ] Run full workspace validation and commit focused stages.

**M5 acceptance:** the source roadmap's first vertical slice works without losing state across daemon restart.

## 8. M6 — Encrypted Google OAuth and manual read-only tools

**Requirement anchors:** encrypted credentials and read-only Gmail/Calendar (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:128-138`).

**Code anchor:** current `TokenStore::save()` writes JSON directly (`crates/corpus/src/tools/mcp/auth.rs:197-205`).

### Task M6.1: Introduce token persistence abstraction

**Files:**
- Modify: `crates/corpus/src/tools/mcp/auth.rs`
- Create: `crates/corpus/src/tools/mcp/token_store.rs`
- Create: `crates/corpus/src/tools/mcp/encrypted_token_store.rs`
- Modify: `crates/corpus/Cargo.toml`

- [ ] Write tests proving ciphertext does not contain access/refresh tokens, wrong keys fail closed, tampering is detected, restrictive file permissions are applied, and legacy plaintext requires explicit migration.
- [ ] Keep OAuth protocol logic in `McpOAuthProvider`; inject token persistence behind a trait.
- [ ] Load the encryption key from an external secret source and never persist it beside ciphertext.
- [ ] Run `cargo test -p corpus -- tools::mcp::token_store tools::mcp::encrypted_token_store`; expect PASS.

### Task M6.2: Configure Google OAuth

**Files:**
- Create: `crates/corpus/src/tools/mcp/google_auth.rs`
- Modify: `crates/corpus/src/tools/mcp/mod.rs`

- [ ] Test Google endpoint/scopes, CSRF state, code exchange, refresh, log redaction, and token-store restart using mock HTTP endpoints.
- [ ] Require read-only scopes initially.
- [ ] Do not enable Google startup if secure persistence is unavailable.
- [ ] Run scoped OAuth tests; expect PASS.

### Task M6.3: Add manual read-only Gmail and Calendar capabilities

**Files:**
- Create or configure provider tools under `crates/corpus/src/tools/google/`
- Modify tool registration at the current registry discovered during milestone planning
- Test with mock MCP/provider endpoints

- [ ] Test Gmail search/read and Calendar list operations through capability admission.
- [ ] Verify refresh tokens and authorization headers never enter tool results or model-visible errors.
- [ ] Expose the read-only capabilities to Telegram chat through the existing ReAct tool path.
- [ ] Run scoped tool tests and a mocked Telegram query.

### Task M6.4: M6 release gate

- [ ] Inspect the token file and prove no plaintext token is present.
- [ ] Query mocked unread mail and today's events through Telegram.
- [ ] Run full workspace validation and commit encryption, OAuth, and tools separately.

**M6 acceptance:** secure Google OAuth survives restart and supports admitted read-only Gmail/Calendar queries.

## 9. M7 — Google sync and Gmail Draft Goal channel

**Requirement anchors:** cursor recovery/dedup (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:140-151`) and Draft Goal behavior (`:153-162`).

### Task M7.1: Add normalized Google event types and cursor repository

**Files:**
- Create: `crates/fabric/src/types/google.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Create: `crates/executive/src/impl/google/cursor.rs`
- Modify: `crates/executive/src/impl/persistence/migrations.rs`

- [ ] Test event serde, stable provider IDs, cursor compare-and-swap, duplicate event suppression, and crash-before/after-dispatch recovery.
- [ ] Persist provider cursor only after normalized events are durably inserted.
- [ ] Run scoped fabric/executive tests; expect PASS.

### Task M7.2: Implement dedicated GoogleSyncManager

**Files:**
- Create: `crates/executive/src/impl/google/mod.rs`
- Create: `crates/executive/src/impl/google/sync.rs`
- Modify: `crates/executive/src/impl/mod.rs`

- [ ] Test Gmail history pagination, Calendar sync-token invalidation/full rescan, rate-limit backoff, cancellation, dedup, and restart.
- [ ] Use a dedicated provider client for background sync; do not route polling through an LLM tool call.
- [ ] Feed normalized events into the durable inbox/event repository.
- [ ] Run `cargo test -p executive -- impl::google`; expect PASS.

### Task M7.3: Implement Gmail channel as untrusted Draft input

**Files:**
- Create: `crates/executive/src/impl/channel/gmail/mod.rs`
- Modify: `crates/executive/src/impl/channel/mod.rs`
- Modify: `crates/executive/src/impl/channel/router.rs`

- [ ] Test allow-listed sender, unknown sender, duplicate message, `[GOAL]`, `[ASK]`, attachments metadata, and spoofing-policy rejection.
- [ ] Map `[GOAL]` only to `GoalState::Draft` and send confirmation to trusted Telegram.
- [ ] Do not execute or approve destructive operations based solely on email sender text.
- [ ] Run Gmail channel tests; expect PASS.

### Task M7.4: M7 release gate

- [ ] Demonstrate cursor restart without duplicate normalized events.
- [ ] Demonstrate `[GOAL]` email creating one Draft and requiring Telegram confirmation.
- [ ] Run full workspace validation and commit cursor, sync, and Gmail channel separately.

**M7 acceptance:** Google sync is restart-safe and Gmail cannot bypass trusted-channel approval.

## 10. M8 — Optional GBrain MemoryService backend

**Requirement anchors:** GBrain stores decisions/outcomes with provenance/freshness and cannot mutate Dasein (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:118-126`).

**Code anchor:** `MemoryService` contract is `crates/mnemosyne/src/service.rs:67-74`; current backend root is `crates/mnemosyne/src/backends/`.

### Task M8.1: Implement client/config/DTOs in the correct backend path

**Files:**
- Create: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/client.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/config.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/types.rs`
- Modify: `crates/mnemosyne/src/backends/mod.rs`
- Modify: `crates/mnemosyne/Cargo.toml`

- [ ] Test health, authentication, timeouts, 4xx/5xx mapping, recall provenance, freshness, and temporal validity with a mock HTTP server.
- [ ] Keep API keys out of logs and model-visible errors.
- [ ] Run `cargo test -p mnemosyne -- backends::gbrain`; expect PASS.

### Task M8.2: Add durable ingestion spool

**Files:**
- Create: `crates/mnemosyne/src/backends/gbrain/spool.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/pipeline.rs`

- [ ] Test daemon restart, GBrain outage, retry/backoff, successful dequeue, malformed-entry dead-lettering, capacity refusal, and no silent drop.
- [ ] Store queued entries in SQLite before acknowledging `record()`.
- [ ] Recover pending rows at service startup.
- [ ] Run spool/pipeline tests; expect PASS.

### Task M8.3: Implement and compose MemoryService

**Files:**
- Create: `crates/mnemosyne/src/backends/gbrain/backend.rs`
- Create: `crates/mnemosyne/src/service/composite.rs`
- Modify: `crates/mnemosyne/src/service.rs` or convert it to a module during the detailed milestone plan
- Modify: daemon memory bootstrap files discovered during milestone planning

- [ ] Add contract tests for `record`, `recall`, `consolidate`, and `forget`.
- [ ] Test degraded recall fallback, recovery, result merging/dedup, and the prohibition on Dasein mutation.
- [ ] Keep the local service available when GBrain is disabled or unhealthy.
- [ ] Run MemoryService contract and integration tests; expect PASS.

### Task M8.4: Add deployment assets

**Files:**
- Create: `docker-compose.gbrain.yml`
- Create: documented example environment file without secrets
- Modify: existing configuration schema and example files discovered during milestone planning

- [ ] Add PostgreSQL and GBrain health checks with dependency ordering.
- [ ] Bind service ports to localhost by default.
- [ ] Validate `docker compose -f docker-compose.gbrain.yml config`; expect exit 0.
- [ ] Run an opt-in integration test against the composed service.

### Task M8.5: M8 release gate

- [ ] Stop GBrain during ingestion and prove queued knowledge survives both process restarts.
- [ ] Restore GBrain and prove the queue drains and recalled entries retain provenance.
- [ ] Run full workspace validation and commit client, spool, service adapter, and deployment assets separately.

**M8 acceptance:** GBrain is an optional, degradable `MemoryService` extension with durable ingestion and no core-runtime dependency.

## 11. M9 — Deployment hardening

**Requirement anchors:** systemd, backups, Tailscale, secret management, health checks, log rotation, and quotas (`docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:178-188`).

- [ ] Add systemd units with restart limits, restricted filesystem access, and explicit secret-file paths.
- [ ] Add backup/restore tests for Goal, inbox, cursor, approval, OAuth ciphertext, and GBrain spool databases.
- [ ] Add health reporting for Telegram, Google OAuth/sync, Goal scheduler, worktree disk budget, and GBrain.
- [ ] Add log redaction regression tests and rotation policy.
- [ ] Bind services to localhost/Tailscale interfaces only; do not expose public unauthenticated ports.
- [ ] Exercise restore on a clean host before declaring production readiness.

**M9 acceptance:** a clean-host restore recovers durable state, secrets remain external, health failures are visible, and disk/process growth is bounded.

## 12. Program-wide traceability

| Design requirement | Implemented by |
|---|---|
| Existing chat remains primary | M0, M1 |
| Reliable Telegram transport | M1 |
| Persistent bounded Goal | M2 |
| Durable attempts and escalation | M3 |
| Pi isolation and evidence | M4 |
| Durable human approval | M5 |
| Encrypted Google read-only integration | M6 |
| Restart-safe sync and Gmail Draft Goals | M7 |
| Optional GBrain backend | M8 |
| Operational recovery and hardening | M9 |

## 13. Explicit prohibitions

- [ ] Do not add Goal-specific variants to `ProcessState`.
- [ ] Do not claim `TokenStore` encryption until the ciphertext tests pass.
- [ ] Do not use `SupervisorTree` as a model router.
- [ ] Do not use one global runtime to represent heterogeneous concurrent workers.
- [ ] Do not run long verification commands inside the context-free `PostTurnPipeline`.
- [ ] Do not use `std::process::Command` in async job paths.
- [ ] Do not acknowledge provider events before durable local commit.
- [ ] Do not silently drop memory ingestion on buffer pressure.
- [ ] Do not allow Gmail alone to authorize execution.
- [ ] Do not make GBrain a prerequisite for the core Goal loop.
- [ ] Do not push, merge, or mutate protected branches automatically.

## 14. Plan self-review checklist

- [ ] Re-read the relevant requirement source and cited code symbols before writing each milestone's detailed execution plan.
- [ ] Replace line ranges if current code moved; never rely on stale anchors.
- [ ] Ensure each detailed plan contains actual test code, implementation code, exact commands, and expected output.
- [ ] Search the detailed plan for `TBD`, `TODO`, “implement later”, and vague error-handling instructions; none may remain.
- [ ] Confirm every new persisted state has migration, restart, conflict, and corruption tests.
- [ ] Confirm every external operation has timeout, cancellation, redaction, and approval behavior.
