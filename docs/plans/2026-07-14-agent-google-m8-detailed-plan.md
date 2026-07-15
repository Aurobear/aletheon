# Aletheon M8 Optional GBrain Memory Detailed Plan

> **For agentic workers:** Preserve local Mnemosyne as the core service. Implement GBrain as an optional supplemental backend over its verified HTTP MCP contract, with a durable local SQLite spool and explicit failure semantics.

**Goal:** Store selected architecture decisions and Goal outcomes in GBrain and recall them with provenance, freshness, and temporal validity without making GBrain a dependency of Goal execution.

**Architecture:** Enrich the existing `MemoryService` DTO contract and compose `DefaultMemoryService` with a supplemental GBrain service. The GBrain transport uses the pinned HTTP MCP operations `query`, `search`, `get_page`, and `put_page`; it never writes GBrain's internal database. Records commit to a local SQLite spool before asynchronous MCP delivery. Recall queries local Mnemosyne first, optionally queries GBrain within an independent budget, then merges bounded normalized results. GBrain has no Dasein write path.

**Tech Stack:** Rust, Tokio, SQLite, serde, existing `MemoryService`, existing Corpus HTTP MCP transport, local Mnemosyne stores, pinned GBrain v0.42.59.0 HTTP MCP daemon.

---

## 1. Requirement, decision, and code anchors

- GBrain must store architecture decisions/Goal outcomes, recall provenance/freshness, project to Agora, and never mutate Dasein: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:118-126`.
- GBrain's verified stable boundary is HTTP MCP, not an internal module/database or an invented REST API: `/home/aurobear/Bear-ws/work/aurb/docs/plans/2026-07-14-gbrain-integration-design.md:39-43,71-72,206-251`.
- The pinned deployment is GBrain release `v0.42.59.0`, commit `5008b287e47bf791132eedfebf66bdef11e9398c`; required operations and source-scoping behavior are documented at `/home/aurobear/Bear-ws/work/aurb/docs/operations/gbrain.md:3-31,56-61` and captured in `/home/aurobear/Bear-ws/work/aurb/config/gbrain/tools-schema.json`.
- User adjudication on 2026-07-15 chose alignment with the real GBrain MCP contract while retaining the M8 SQLite spool, local fallback, temporal metadata, health, and dead-letter requirements.
- Current `MemoryService` exposes only `record/recall/consolidate/forget`, and `RecallSet` is plain strings: `crates/mnemosyne/src/service.rs:25-74`.
- `DefaultMemoryService` is the current SQLite-backed local implementation: `crates/mnemosyne/src/service.rs:76-168`.
- Daemon bootstrap constructs that concrete local service: `crates/executive/src/impl/daemon/handler/init.rs:896-902`.
- Current GBrain recall bypasses `MemoryService` and injects MCP results directly from the turn pipeline: `crates/executive/src/service/daemon_turn/gbrain.rs:36-166`.
- Current GBrain writes use a directory-based JSON outbox rather than the required SQLite spool: `crates/executive/src/service/daemon_turn/gbrain.rs:460-711`.
- Generic authenticated HTTP MCP connectivity already exists in Corpus and must be reused rather than duplicated: `crates/corpus/src/tools/mcp/manager.rs` and `crates/corpus/src/tools/mcp/transport.rs`.
- The real Mnemosyne backend root is `crates/mnemosyne/src/backends/`; M8 must not create the obsolete `src/impl/backends` path.

**Migration decision:** Keep the verified HTTP MCP transport. Move GBrain governance behind the enriched `MemoryService`, replace the legacy JSON outbox with a SQLite spool, migrate/replay legacy entries explicitly, and remove the direct turn-pipeline bypass only after equivalent recall tests pass.

## 2. Task 1 — Enrich the MemoryService contract compatibly

**Files:**

- Modify: `crates/mnemosyne/src/service.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`

- [ ] Add contract tests first for provenance, source time, observed time, valid-from/valid-until, supersession, confidence, sensitivity, and stable record ID.
- [ ] Replace string-only recall rows with `RecallItem`; keep a compatibility helper that extracts text for existing consumers during migration.
- [ ] Enrich `ExperienceEvent` with typed message/reflection/architecture-decision/Goal-outcome records and metadata rather than adding an unrelated backend-only DTO.
- [ ] Define temporal state as Current, Superseded, Expired, or Unknown based on explicit fields; do not infer “obsolete” solely from age.
- [ ] Require bounded query, result count, content bytes, optional current-at timestamp, and explicit historical inclusion in `RecallRequest`.
- [ ] Update `DefaultMemoryService` mappings and existing tests without changing its local durability behavior.
- [ ] Run `cargo test -p mnemosyne -- service`; expect PASS.
- [ ] Commit `feat(mnemosyne): preserve provenance in memory service`.

## 3. Task 2 — Pin the GBrain MCP contract and configuration

**Files:**

- Create: `config/gbrain/tools-schema.json`
- Create: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/config.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/page.rs`
- Modify: `crates/mnemosyne/src/backends/mod.rs`
- Modify: `crates/cognit/src/config/mod.rs`
- Modify: `config/default.toml`

- [ ] Copy the verified `tools/list` fixture for GBrain v0.42.59.0 and assert the exact required operation schemas for `query`, `search`, `get_page`, and `put_page`; ignore unrelated additive operations but reject missing or incompatible required fields.
- [ ] Define supplemental GBrain configuration with enabled flag, MCP server name, read/write source policy, request timeout, batch size, recall limit, retry policy, spool limits, schema fixture/version, and legacy-outbox migration path.
- [ ] Reuse Corpus MCP server authentication configuration and secret-file/token handling; do not add a second bearer-token parser or put credentials in Mnemosyne DTOs.
- [ ] Define a versioned deterministic Markdown page contract mapping `ExperienceEvent` to `put_page {slug,content}` and MCP search/get-page responses to `RecallItem`.
- [ ] Respect verified source behavior: `query` may set `source_id`; `get_page` and `put_page` rely on token/operation context and never invent a source parameter.
- [ ] Reject missing provenance, invalid timestamps, oversized pages, unknown mandatory page schema versions, and server-returned identity/Dasein mutation instructions.
- [ ] Run `cargo test -p mnemosyne -- backends::gbrain::config backends::gbrain::page`; expect PASS.
- [ ] Commit `feat(mnemosyne): define GBrain MCP page contract`.

## 4. Task 3 — Implement the bounded HTTP MCP adapter

**Files:**

- Create: `crates/executive/src/impl/gbrain/mod.rs`
- Create: `crates/executive/src/impl/gbrain/mcp_adapter.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Test: `crates/executive/tests/gbrain_mcp_adapter.rs`

- [ ] Add fake HTTP MCP tests for initialize/tools-list validation, `put_page` success/idempotency, `query`/`search`, optional `get_page`, auth failure, timeout, cancellation, 429/5xx transport failure, malformed JSON/tool content, oversized response, source scoping, and redacted errors.
- [ ] Reuse one retained `corpus::tools::mcp::manager::McpManager`; do not add another HTTP/JSON-RPC implementation.
- [ ] Validate the pinned required tool schemas before enabling supplemental recall or delivery; schema drift marks GBrain degraded and leaves local memory operational.
- [ ] Classify connection/timeout/rate/provider failures as transient; classify auth, schema, invalid page, and rejected tool arguments as operator/permanent errors.
- [ ] Normalize only bounded text and explicit metadata from MCP results; discard remote control fields and never pass tool directives into prompts.
- [ ] Expose sanitized health containing state, last success/error category, consecutive failures, schema status, and queue depth.
- [ ] Run `cargo test -p executive --test gbrain_mcp_adapter`; expect PASS.
- [ ] Commit `feat(executive): add bounded GBrain MCP adapter`.

## 5. Task 4 — Add durable SQLite ingestion spool

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/spool.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/migrations.rs`
- Test: `crates/mnemosyne/tests/gbrain_spool.rs`

- [ ] Add SQLite tables for normalized page records, attempts, lease/next-attempt, delivery receipt, and dead letters; stable record ID/slug is unique.
- [ ] `enqueue` commits normalized page payload and hash before returning success. Duplicate enqueue with identical hash is idempotent; same ID/different hash is a conflict.
- [ ] Implement atomic claim with expiring lease, bounded exponential backoff with jitter, maximum attempts/age, receipt acknowledgement, and permanent-failure dead-letter transition.
- [ ] Treat successful MCP `put_page` plus a crash before local acknowledgement as safe redelivery under the deterministic slug/content hash contract.
- [ ] Enforce spool item/byte quotas. On exhaustion, preserve core Goal execution and return explicit degraded-memory health rather than silently dropping.
- [ ] Do not store secrets; apply sensitivity policy before enqueue and exclude sensitive records rather than storing plaintext sensitive content.
- [ ] Import legacy JSON outbox entries through an explicit bounded migration that verifies/redacts each page, is restart-idempotent, and never deletes a source file before SQLite commit.
- [ ] Test database crash/reopen, process crash after claim/delivery/before ack, concurrent workers, lease takeover, duplicate receipt, corruption, disk-full injection, dead-letter inspection/requeue, and legacy migration.
- [ ] Run `cargo test -p mnemosyne --test gbrain_spool`; expect PASS.
- [ ] Commit `feat(mnemosyne): spool GBrain pages durably`.

## 6. Task 5 — Implement the supplemental backend and worker lifecycle

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/backend.rs`
- Create: `crates/executive/src/impl/gbrain/worker.rs`
- Modify: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Test: `crates/mnemosyne/tests/gbrain_backend_contract.rs`
- Test: `crates/executive/tests/gbrain_worker.rs`

- [ ] Define a transport-neutral supplemental-memory trait in Mnemosyne; Executive's MCP adapter implements the transport boundary so Mnemosyne does not depend on Corpus.
- [ ] `record` validates and enqueues locally; it does not wait for MCP network success.
- [ ] Worker drains bounded batches to MCP `put_page`, honors cancellation/shutdown, persists retry state, and resumes on startup. Permanent malformed entries become inspectable dead letters.
- [ ] `recall` has a strict latency budget and uses `query` for configured sources, falls back to verified `search` behavior when appropriate, and calls `get_page` only for selected hits lacking bounded content.
- [ ] Unavailable/slow GBrain returns an empty supplemental set plus health evidence, not an error that aborts a Goal.
- [ ] `forget` remains explicit Unsupported until the approved memory policy adopts and tests GBrain `delete_page`; never pretend deletion succeeded.
- [ ] Run the supplemental contract suite for provenance/temporal fields, outage behavior, and stable page identity.
- [ ] Run `cargo test -p mnemosyne --test gbrain_backend_contract` and `cargo test -p executive --test gbrain_worker`; expect PASS.
- [ ] Commit `feat(executive): run optional GBrain memory worker`.

## 7. Task 6 — Compose local Mnemosyne and GBrain memory services

**Files:**

- Create: `crates/mnemosyne/src/composite_service.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/src/core/memory_group.rs`
- Modify: `crates/executive/src/service/daemon_turn/gbrain.rs`
- Test: `crates/executive/tests/gbrain_bootstrap.rs`

- [ ] `CompositeMemoryService` always records to `DefaultMemoryService` first, then enqueues only policy-selected records to the supplemental service.
- [ ] Selection initially includes approved architecture decisions and terminal Goal summaries; raw chat, credentials, sensitive email bodies, transient tool output, and Dasein/self-memory are excluded.
- [ ] Recall queries local Mnemosyne and GBrain within independent budgets, normalizes/merges by stable provenance key, prefers the newest valid version, and retains superseded entries only when explicitly requested.
- [ ] Bootstrap constructs local-only when disabled/misconfigured and composite when enabled; MCP/schema/startup failure marks optional degraded health but daemon initialization succeeds.
- [ ] Remove the direct `TurnPipeline::inject_gbrain_recall` bypass after equivalent composite recall tests pass; keep only bounded rendering/compatibility helpers that consume `MemoryService` results.
- [ ] Shutdown stops intake, leaves committed spool work durable, cancels the worker, and bounds worker join time.
- [ ] Test disabled, healthy, unavailable-at-start, schema drift, outage-after-start, slow recall, duplicate local/remote fact, malformed remote item, legacy config migration, and shutdown with queued entries.
- [ ] Run `cargo test -p executive --test gbrain_bootstrap`; expect PASS.
- [ ] Commit `feat(executive): compose optional GBrain memory`.

## 8. Task 7 — Record decisions and Goal outcomes

**Files:**

- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/goal/verification.rs`
- Create: `crates/executive/src/impl/memory_projection.rs`
- Test: `crates/executive/tests/goal_memory_projection.rs`

- [ ] Project only persisted terminal/approved Goal summaries and explicitly approved architecture decisions.
- [ ] Include Goal/attempt/artifact/approval IDs, source commit, outcome, verification evidence, created/valid times, principal/sensitivity, and supersedes relation.
- [ ] Use deterministic record IDs and page slugs so restart/replayed completion events cannot duplicate GBrain pages.
- [ ] A newer architecture decision references the old decision as superseded; both remain recallable and current-only recall returns the valid new one.
- [ ] Projection failure/degraded spool never changes the Goal's terminal result; record sanitized health/audit evidence instead.
- [ ] Test successful/failed/rejected Goals, duplicate terminal events, revised decision, stale decision query, sensitive exclusions, and GBrain outage.
- [ ] Run `cargo test -p executive --test goal_memory_projection`; expect PASS.
- [ ] Commit `feat(executive): project durable outcomes to memory`.

## 9. Task 8 — Bound recall projection into Agora

**Files:**

- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/impl/hook_lifecycle/recall_inject.rs`
- Modify: `crates/executive/src/service/daemon_turn/gbrain.rs`
- Test: `crates/executive/tests/gbrain_recall_injection.rs`

- [ ] Convert merged `MemoryService` recall items into a bounded context section labeled with source, observed time, valid interval, current/superseded state, and confidence.
- [ ] Default query requests Current entries only; historical queries may include superseded/expired results explicitly.
- [ ] Enforce item, byte, and latency budgets and deterministic ranking; GBrain text is untrusted reference data, not instructions.
- [ ] Never inject credentials, high-sensitivity excluded records, Dasein mutations, remote tool directives, or raw MCP envelopes.
- [ ] Prove the turn pipeline uses only `MemoryService::recall`; no independent GBrain query path remains.
- [ ] Test prompt-injection-shaped memory, stale/current conflicts, provenance rendering, byte truncation, timeout, empty supplemental recall, local fallback, and MCP schema drift.
- [ ] Run `cargo test -p executive --test gbrain_recall_injection`; expect PASS.
- [ ] Commit `feat(executive): inject bounded composite recall`.

## 10. Task 9 — Add local GBrain deployment assets

**Files:**

- Create: `deploy/gbrain/compose.yaml`
- Create: `deploy/gbrain/.env.example`
- Create: `deploy/gbrain/README.md`
- Modify: `config/default.toml`

- [ ] Pin GBrain v0.42.59.0 commit `5008b287e47bf791132eedfebf66bdef11e9398c` and document the exact captured MCP tools schema used by contract fixtures.
- [ ] Bind HTTP MCP only to loopback or an internal container network; configure persistent brain/database volumes, healthcheck, restart policy, resource limits, and log rotation.
- [ ] Keep read/write credentials outside Git and reference secret-file mounts; `.env.example` contains placeholders only and documents separate least-privilege tokens.
- [ ] Document initialize/tools-list validation, source binding, upgrade, backup, restore, health, SQLite dead-letter inspection, legacy-outbox migration, and full disable/local fallback.
- [ ] Validate `docker compose config`; start against a disposable volume and run MCP contract smoke tests.
- [ ] Commit `docs(deploy): add local GBrain MCP service assets`.

## 11. M8 release audit

- [ ] Run formatting, all scoped tests, workspace tests, and workspace build.
- [ ] Prove GBrain down/schema-incompatible at startup and mid-Goal does not block Telegram, Goal execution, approval, or local Mnemosyne recall.
- [ ] Kill daemon after enqueue and GBrain after `put_page` succeeds but before spool acknowledgement; restart both and prove one logical remote page with no silent drop.
- [ ] Exhaust retry for malformed/auth/schema failures and prove inspectable dead-letter/operator state without retry storm.
- [ ] Record two architecture decisions with a supersedes edge and prove current-only and historical recall differ correctly.
- [ ] Audit MCP requests, SQLite spool, legacy migration, logs, and injected context for tokens, raw sensitive mail, raw MCP envelopes, and Dasein records.
- [ ] Restore GBrain data volume and Aletheon spool from backup and verify delivery/recall consistency.
- [ ] Compare the checked-in required-operation schema with a live pinned `tools/list` response; fail release on incompatible drift.

## 12. Implementation batches

1. Tasks 1–3: enriched memory contract, pinned page/schema/config contract, bounded MCP adapter.
2. Tasks 4–5: SQLite spool, legacy migration, supplemental backend, worker lifecycle.
3. Tasks 6–8: local-first composition, durable projections, unified bounded recall injection.
4. Tasks 9–11: pinned deployment, outage/restart/schema-drift audit.

Guardrails:

```text
Do not replace or bypass DefaultMemoryService.
Do not let GBrain outage or schema drift fail core Goal execution.
Do not acknowledge ingestion before local SQLite spool commit.
Do not flatten provenance or temporal validity into plain text.
Do not send Dasein/self-memory, credentials, sensitive raw mail, or raw transcripts.
Do not invent REST endpoints or unverified MCP arguments.
Do not write GBrain's internal PGLite/Postgres directly.
Stop after each batch with outage/restart evidence.
```
