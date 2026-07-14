# Aletheon M7 Google Sync and Gmail Channel Detailed Plan

> **For agentic workers:** Land durable synchronization before enabling Gmail-triggered Goals. Every cursor transition and channel action requires restart/replay tests.

**Goal:** Resume Gmail, Calendar, and selected Drive synchronization after restart without duplicate effects, then accept authenticated `[GOAL]` mail as a Draft that requires trusted Telegram approval.

**Architecture:** A dedicated `GoogleSyncManager` fetches provider deltas into an external SQLite projection. In one transaction it inserts a deduplicated normalized event/outbox record and advances the cursor; downstream consumers update projections, wake Goals, or notify Telegram idempotently. Gmail input is a channel adapter over those durable events, not a second polling loop.

**Tech Stack:** Rust, Tokio, SQLite, Gmail history API, Calendar sync tokens, Drive changes API, existing ObjectiveStore/Goal lifecycle, durable channel inbox/outbox, Telegram approvals, artifact store.

---

## 1. Requirement and code anchors

- Sync requires Gmail history, Calendar sync tokens, Drive cursors, retries, deduplication, normalized events, and projections: `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:124-144`.
- Gmail task subjects and Draft/AwaitingHuman default are specified at `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:146-170`.
- Normalized event families are specified at `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:205-221`.
- External/provider data must stay outside Agora/Mnemosyne unless deliberately projected: `docs/arch/agent-google/02_GOOGLE_ECOSYSTEM_INTEGRATION.md:223-239`.
- M7 restart acceptance and Gmail channel acceptance are at `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:140-162`.
- M2 startup recovery already reloads nonterminal Goals from ObjectiveStore: `crates/executive/src/impl/daemon/handler/init.rs:249-278`.
- M5 approval is durable while `SocketApprovalGate` remains synchronous; follow `docs/plans/2026-07-14-agent-google-m5-detailed-plan.md:1-16`.

**Scope decision:** Gmail and Calendar incremental sync plus Gmail Draft intake are release blockers. Drive change-cursor support is implemented for explicitly selected files/roots, but file-content ingestion remains opt-in and bounded. Outgoing Gmail send and Calendar writes remain disabled until separate write grants and M5 approvals exist.

## 2. Task 1 — Define normalized Google events

**Files:**

- Modify: `crates/fabric/src/types/google.rs`
- Create: `crates/fabric/src/types/external_event.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Test stable serialization and dedup keys before implementing event types.
- [ ] Define `ExternalEventId`, `ExternalObjectRef`, `GoogleEvent`, mail/calendar/drive change variants, and `ExternalEventEnvelope`.
- [ ] Envelope contains provider/account, provider event ID or deterministic fallback key, object version, observed/source timestamps, provenance, payload hash, and schema version.
- [ ] Separate metadata events from bounded content/artifact references; never embed credentials or unlimited mail/file bodies.
- [ ] Define deterministic dedup key as provider + account + event kind + provider event/object version, not arrival time.
- [ ] Run `cargo test -p fabric -- types::external_event types::google`; expect PASS.
- [ ] Commit `feat(fabric): define normalized Google events`.

## 3. Task 2 — Add external projection, cursor, and outbox tables

**Files:**

- Modify: `crates/executive/src/impl/goal/migrations.rs`
- Create: `crates/executive/src/impl/google/mod.rs`
- Create: `crates/executive/src/impl/google/store.rs`
- Modify: `crates/executive/src/impl/mod.rs`

- [ ] Add `google_sync_cursors`, `google_events`, `google_objects`, `google_subscriptions`, and `google_event_outbox` tables.
- [ ] Uniqueness covers `(account_id, stream, dedup_key)`; cursors store opaque token, generation, last-success/error time, retry state, and optimistic version.
- [ ] Implement one transaction that inserts event/projection/outbox and advances cursor only after all durable writes succeed.
- [ ] Duplicate events return the existing event and may advance only to a proven successor cursor; a failed insert/dispatch never loses the cursor position.
- [ ] Store sensitive provider payloads bounded and encrypted when they contain mail/file content; projection rows retain provenance and deletion tombstones.
- [ ] Test rollback at each statement, duplicate delivery, cursor compare-and-swap, out-of-order events, restart, tombstones, and concurrent pollers.
- [ ] Run `cargo test -p executive -- impl::google::store`; expect PASS.
- [ ] Commit `feat(executive): persist Google sync state`.

## 4. Task 3 — Implement Gmail history synchronization

**Files:**

- Create: `crates/corpus/src/tools/google/gmail_sync.rs`
- Modify: `crates/corpus/src/tools/google/gmail.rs`
- Test: `crates/corpus/tests/gmail_history_sync.rs`

- [ ] Add fixture tests for initial baseline, paginated history, additions/updates/deletions, duplicate pages, expired history ID, 401 refresh, 429, and cancellation.
- [ ] Initial enrollment records a baseline profile/history ID before emitting later deltas; it does not treat the entire mailbox as new input.
- [ ] Fetch changed message metadata separately and enforce per-run page/message/body limits.
- [ ] On expired history ID, perform a bounded reconciliation of configured labels/query, deduplicate against projections, then establish a new baseline; emit a health event if bounds prevent completion.
- [ ] Return a batch with input cursor, ordered normalized events, and successor cursor; Corpus never writes the executive database itself.
- [ ] Run `cargo test -p corpus --test gmail_history_sync`; expect PASS.
- [ ] Commit `feat(corpus): synchronize Gmail history`.

## 5. Task 4 — Implement Calendar and selected Drive synchronization

**Files:**

- Create: `crates/corpus/src/tools/google/calendar_sync.rs`
- Create: `crates/corpus/src/tools/google/drive.rs`
- Create: `crates/corpus/src/tools/google/drive_sync.rs`
- Modify: `crates/corpus/src/tools/google/mod.rs`
- Test: `crates/corpus/tests/google_delta_sync.rs`

- [ ] Calendar tests cover initial bounded window, recurring instances, cancellations/tombstones, pagination, sync-token continuation, and HTTP 410 invalidation.
- [ ] On Calendar 410, rebuild only the configured time window, deduplicate by event ID/version, and replace the token after durable reconciliation.
- [ ] Drive tests cover start-page token, changes pagination, deletion, shared-drive flags, unselected files, MIME/size policy, and expired cursor recovery.
- [ ] Drive emits metadata for all selected-scope changes but downloads content only when selection, MIME allowlist, and byte cap all pass; content becomes an artifact reference.
- [ ] Request Drive read-only scope incrementally only when Drive sync is enabled; M7 must still operate for Gmail/Calendar-only accounts.
- [ ] Run `cargo test -p corpus --test google_delta_sync`; expect PASS.
- [ ] Commit `feat(corpus): synchronize Calendar and Drive changes`.

## 6. Task 5 — Build GoogleSyncManager lifecycle

**Files:**

- Create: `crates/executive/src/impl/google/sync_manager.rs`
- Create: `crates/executive/src/impl/google/event_dispatcher.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/google_sync_recovery.rs`

- [ ] Start one supervised poll loop per active account/stream and acquire a database lease so multiple daemon instances cannot poll the same cursor concurrently.
- [ ] Use bounded exponential backoff with jitter for transient failures, `Retry-After` when valid, circuit-open health state for repeated failures, and immediate stop on revocation/auth-required.
- [ ] Persist retry/health state; shutdown waits for the current database transaction, not an entire network retry.
- [ ] Dispatcher claims durable outbox rows, delivers idempotently, and marks completion. A crash between delivery and acknowledgement may redeliver but must not duplicate downstream effects.
- [ ] Startup restores accounts/cursors/outbox before starting pollers, then resumes due work.
- [ ] Test crash before/after cursor commit, before/after dispatch acknowledgement, lease expiry/takeover, two managers, revocation, offline recovery, and graceful shutdown.
- [ ] Run `cargo test -p executive --test google_sync_recovery`; expect PASS.
- [ ] Commit `feat(executive): supervise durable Google sync`.

## 7. Task 6 — Route events to projections, Goals, and Telegram

**Files:**

- Modify: `crates/executive/src/impl/google/event_dispatcher.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/channel/router.rs`
- Test: `crates/executive/tests/google_event_routing.rs`

- [ ] Define subscription rules by principal/account/event kind/query and store their version with the cursor generation.
- [ ] Calendar changes may wake a waiting Goal only through an explicit persisted wait condition matching event ID/time predicate.
- [ ] Important-mail/calendar notifications go through durable channel outbox with event ID as idempotency key.
- [ ] Projection into Agora is bounded/current-task-only; long-term-memory proposals retain provenance and wait for M8 policy ingestion.
- [ ] Deleted/updated provider facts create tombstones/new versions instead of silently rewriting historical facts.
- [ ] Test unmatched events, multiple subscriptions, duplicate delivery, Goal already terminal, notification retry, stale updates, and revoked accounts.
- [ ] Run `cargo test -p executive --test google_event_routing`; expect PASS.
- [ ] Commit `feat(executive): route normalized Google events`.

## 8. Task 7 — Validate Gmail senders and classify subjects

**Files:**

- Create: `crates/executive/src/impl/channel/gmail/mod.rs`
- Create: `crates/executive/src/impl/channel/gmail/sender_policy.rs`
- Create: `crates/executive/src/impl/channel/gmail/classifier.rs`
- Modify: `crates/executive/src/impl/channel/mod.rs`
- Test: `crates/executive/tests/gmail_channel_policy.rs`

- [ ] Configure exact-address/domain allowlists per bound principal; default is deny.
- [ ] Parse RFC message headers defensively and require aligned From plus a trusted receiving-chain `Authentication-Results` result for SPF or DKIM according to configured policy. If the trusted header cannot be identified, fail closed.
- [ ] Do not treat display names, Reply-To, forwarded body text, or an untrusted injected `Authentication-Results` header as identity proof.
- [ ] Classify only exact bounded subject prefixes `[ASK]`, `[GOAL]`, `[MEMORY]`, `[DOC]`; unknown/unverified mail becomes notification/quarantine only.
- [ ] Persist verified sender principal, policy version, message ID, thread ID, classification, and evidence hash in the durable inbox.
- [ ] Test spoofed From, look-alike Unicode/domain, multiple From headers, forwarded mail, header injection, SPF/DKIM failure, allowlist changes, duplicate message, and oversized headers.
- [ ] Run `cargo test -p executive --test gmail_channel_policy`; expect PASS.
- [ ] Commit `feat(executive): authenticate Gmail channel input`.

## 9. Task 8 — Ingest bounded mail bodies and attachments

**Files:**

- Create: `crates/executive/src/impl/channel/gmail/ingest.rs`
- Modify: `crates/executive/src/impl/channel/gmail/mod.rs`
- Modify: existing artifact-store integration selected during implementation
- Test: `crates/executive/tests/gmail_attachment_ingest.rs`

- [ ] Prefer text/plain, sanitize HTML to bounded text, strip quoted history/signatures by deterministic conservative rules, and retain the original message as an external reference.
- [ ] Enforce total/message/attachment count and byte caps before download; allowlist MIME types and reject archives, executables, macros, path traversal, nested multipart bombs, and declared/actual type mismatch.
- [ ] Stream accepted attachments into content-addressed artifacts with hash, size, MIME, provider IDs, source timestamps, and quarantine/scan status.
- [ ] Unscanned or rejected attachments are never passed to tools/model; Goal draft lists them as unavailable evidence.
- [ ] Test malformed MIME, duplicate attachments, huge/unknown length, partial download, cancellation, filename traversal, HTML active content, and restart midway.
- [ ] Run `cargo test -p executive --test gmail_attachment_ingest`; expect PASS.
- [ ] Commit `feat(executive): ingest bounded Gmail artifacts`.

## 10. Task 9 — Create Draft Goals and require Telegram confirmation

**Files:**

- Modify: `crates/executive/src/impl/channel/gmail/mod.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/channel/telegram/mod.rs`
- Test: `crates/executive/tests/gmail_goal_draft.rs`

- [ ] A verified `[GOAL]` event creates exactly one ObjectiveStore Goal in `Draft`, owned by the bound principal, with source event/message/artifact references and no scheduled job.
- [ ] Unverified mail, unsupported classification, ingestion failure, duplicate event, or missing principal never creates an executable Goal.
- [ ] Enqueue a Telegram review containing bounded intent/source/risk/artifact summary and actions Confirm/Edit/Reject.
- [ ] Confirm uses the M5 durable approval/owner-binding path, transitions Draft to active planning, and records Telegram identity/channel. Email replies cannot self-approve.
- [ ] Edit creates a versioned revised intent and requires a fresh confirmation; Reject terminates the draft without executing it.
- [ ] Test duplicate history events, replayed callbacks, wrong Telegram user, restart before confirmation, edited draft, revoked sender/account, and confirmation racing deletion.
- [ ] Run `cargo test -p executive --test gmail_goal_draft`; expect PASS.
- [ ] Commit `feat(executive): gate emailed goals as drafts`.

## 11. Task 10 — Add controlled report delivery boundary

**Files:**

- Create: `crates/executive/src/impl/channel/gmail/report.rs`
- Modify: `crates/executive/src/impl/channel/gmail/mod.rs`
- Test: `crates/executive/tests/gmail_report_policy.rs`

- [ ] M7 default produces a local report artifact and Telegram notification only; no Gmail send tool is registered under read-only grants.
- [ ] If a later incremental `gmail.compose`/`gmail.send` grant exists, draft creation or sending is a separate M5 approval category bound to recipient, subject, body/artifact hashes, and account.
- [ ] Never reuse inbound read authorization as send authorization and never infer recipients from untrusted message body content.
- [ ] Delivery outbox uses approval ID plus immutable report hash for idempotency; ambiguous provider timeout requires reconciliation before retry.
- [ ] Test read-only denial, missing/expired/replayed approval, changed recipient/body, provider timeout, duplicate send, and revoked write grant.
- [ ] Run `cargo test -p executive --test gmail_report_policy`; expect PASS.
- [ ] Commit `feat(executive): enforce Gmail report boundary`.

## 12. M7 release audit

- [ ] Run `cargo fmt --all -- --check`, all scoped tests, `cargo test --workspace`, and `cargo build --workspace`.
- [ ] Run a restart matrix at every fetch/store/outbox/delivery boundary; prove one logical provider event has at most one projection, Goal draft, wake-up, and Telegram notification.
- [ ] Force Gmail history expiry and Calendar token 410; prove bounded reconciliation neither loses nor duplicates known events.
- [ ] Disconnect network, restart daemon, restore network, and prove persisted backoff/cursors recover.
- [ ] Send spoofed and valid `[GOAL]` fixtures; prove only the authenticated allowlisted sender creates Draft and neither starts execution before Telegram confirmation.
- [ ] Prove attachment limits/quarantine prevent untrusted bytes entering model/tool context.
- [ ] Prove Gmail send and Calendar write remain unavailable under read-only grants.
- [ ] Audit SQLite, logs, events, artifacts, Telegram, and memory proposals for tokens and unbounded sensitive payloads.

## 13. DeepSeek batches

1. Tasks 1–2: events and durable sync store.
2. Tasks 3–5: provider deltas and manager lifecycle.
3. Tasks 6–8: routing and Gmail channel ingestion.
4. Tasks 9–12: Goal confirmation, report boundary, recovery audit.

Guardrails:

```text
Do not advance a cursor before event/outbox persistence commits.
Do not add a second Gmail poller for the channel.
Do not trust From alone or untrusted Authentication-Results headers.
Do not execute [GOAL] mail before owner Telegram confirmation.
Do not download unselected or unbounded Drive/attachment content.
Do not enable send/write with read-only grants.
Stop after each batch with restart and dedup evidence.
```
