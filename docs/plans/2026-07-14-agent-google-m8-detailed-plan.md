# Aletheon M8 Optional GBrain Memory Detailed Plan

> **For agentic workers:** Preserve local memory as the core service. Implement GBrain as an optional supplemental backend with a durable local spool and explicit failure semantics.

**Goal:** Store selected architecture decisions and Goal outcomes in GBrain and recall them with provenance, freshness, and temporal validity without making GBrain a dependency of Goal execution.

**Architecture:** Enrich the existing `MemoryService` DTO contract, add a GBrain REST client and SQLite spool under Mnemosyne, and compose it with `DefaultMemoryService`. Records commit locally to the spool before remote delivery; recall merges bounded local and remote results. GBrain may supplement Agora context but has no Dasein write path.

**Tech Stack:** Rust, Tokio, reqwest, SQLite, serde, existing `MemoryService`, local Mnemosyne stores, external GBrain container/API.

---

## 1. Requirement and code anchors

- GBrain must store architecture decisions/Goal outcomes, recall provenance/freshness, project to Agora, and never mutate Dasein: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:118-126`.
- The approved design requires a REST client, health state, DTO mapping, SQLite ingestion spool, local fallback, bounded retry, and dead-letter state: `docs/plans/2026-07-14-agent-google-design.md:287-299`.
- Release requires contract compatibility, outage independence, restart-safe ingestion, and temporal metadata: `docs/plans/2026-07-14-agent-google-design.md:353-358`.
- Current `MemoryService` exposes only `record/recall/consolidate/forget`, and `RecallSet` is plain strings: `crates/mnemosyne/src/service.rs:25-74`.
- `DefaultMemoryService` is the current SQLite-backed implementation: `crates/mnemosyne/src/service.rs:76-168`.
- Daemon bootstrap constructs that concrete service directly: `crates/executive/src/impl/daemon/handler/init.rs:546-552`.
- The real backend root is `crates/mnemosyne/src/backends/`; M8 must not create the obsolete `src/impl/backends` path.

## 2. Task 1 — Enrich the MemoryService contract compatibly

**Files:**

- Modify: `crates/mnemosyne/src/service.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`

- [ ] Add contract tests first for provenance, source time, observed time, valid-from/valid-until, supersession, confidence, sensitivity, and stable record ID.
- [ ] Replace string-only recall rows with `RecallItem`; keep a compatibility helper that extracts text for existing consumers during migration.
- [ ] Enrich `ExperienceEvent` with typed message/reflection/architecture-decision/Goal-outcome records and metadata rather than adding an unrelated backend-only DTO.
- [ ] Define temporal state as Current, Superseded, Expired, or Unknown based on explicit fields; do not infer “obsolete” solely from age.
- [ ] Require bounded query, result count, content bytes, and optional current-at timestamp in `RecallRequest`.
- [ ] Update `DefaultMemoryService` mappings and existing tests without changing its local durability behavior.
- [ ] Run `cargo test -p mnemosyne -- service`; expect PASS.
- [ ] Commit `feat(mnemosyne): preserve provenance in memory service`.

## 3. Task 2 — Define GBrain configuration and wire protocol

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/config.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/dto.rs`
- Modify: `crates/mnemosyne/src/backends/mod.rs`
- Modify: `crates/executive/src/core/config/mod.rs`
- Modify: `crates/executive/src/core/config/infra.rs`
- Modify: `config/default.toml`

- [ ] Define `GBrainConfig` with enabled flag, base URL, credential-file path, connect/request timeout, batch size, recall limit, retry policy, and spool limits.
- [ ] Reject public/non-HTTPS endpoints unless an explicit loopback/private-container development mode is enabled.
- [ ] Read API credentials from a restrictive secret file at request time/startup; redact URL query, headers, payloads, and credentials from Debug/errors.
- [ ] Define versioned ingest/search request/response DTOs and strict conversion to/from `ExperienceEvent`/`RecallItem`.
- [ ] Reject missing provenance, invalid timestamps, oversized records, unknown mandatory schema versions, and server-returned identity/Dasein mutation instructions.
- [ ] Use fixture tests matching the deployed GBrain API version; if the API contract differs, stop and update this plan before implementation rather than guessing fields.
- [ ] Run `cargo test -p mnemosyne -- backends::gbrain::config backends::gbrain::dto`; expect PASS.
- [ ] Commit `feat(mnemosyne): define GBrain protocol contract`.

## 4. Task 3 — Implement bounded GBrain REST client

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/client.rs`
- Modify: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Modify: `crates/mnemosyne/Cargo.toml` only if test/server dependencies are missing
- Test: `crates/mnemosyne/tests/gbrain_client.rs`

- [ ] Add mock-server tests for ingest success/idempotency, search, auth failure, timeout, cancellation, 429 Retry-After, 5xx, malformed JSON, oversized response, and redacted errors.
- [ ] Use one async `reqwest::Client`, finite timeouts, response-byte caps, explicit status classification, and request IDs/idempotency keys derived from stable record IDs.
- [ ] Treat network/408/429/5xx as transient; treat schema/auth/most 4xx as permanent or operator-action-required.
- [ ] Search returns bounded normalized items, discards invalid rows with a health counter, and never passes remote control fields into prompts.
- [ ] Expose health snapshot containing state, last success/error category, consecutive failures, and queue depth but no sensitive content.
- [ ] Run `cargo test -p mnemosyne --test gbrain_client`; expect PASS.
- [ ] Commit `feat(mnemosyne): add GBrain REST client`.

## 5. Task 4 — Add durable ingestion spool

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/spool.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/migrations.rs`
- Test: `crates/mnemosyne/tests/gbrain_spool.rs`

- [ ] Add SQLite tables for records, attempts, lease/next-attempt, delivery receipt, and dead letters; stable record ID is unique.
- [ ] `enqueue` commits the normalized payload and hash before returning success. Duplicate enqueue with identical hash is idempotent; same ID/different hash is a conflict.
- [ ] Implement atomic claim with expiring lease, bounded exponential backoff with jitter, maximum attempts/age, receipt acknowledgement, and permanent-failure dead-letter transition.
- [ ] Enforce spool item/byte quotas. On quota exhaustion, preserve core Goal execution and return an explicit degraded-memory result/metric rather than silently dropping.
- [ ] Do not store secrets; apply sensitivity policy before enqueue and encrypt locally queued sensitive content if such records are enabled.
- [ ] Test database crash/reopen, process crash after claim/delivery/before ack, concurrent workers, lease takeover, duplicate receipt, corruption, disk-full injection, and dead-letter inspection/requeue.
- [ ] Run `cargo test -p mnemosyne --test gbrain_spool`; expect PASS.
- [ ] Commit `feat(mnemosyne): spool GBrain ingestion durably`.

## 6. Task 5 — Implement GBrainBackend and worker lifecycle

**Files:**

- Create: `crates/mnemosyne/src/backends/gbrain/backend.rs`
- Create: `crates/mnemosyne/src/backends/gbrain/worker.rs`
- Modify: `crates/mnemosyne/src/backends/gbrain/mod.rs`
- Test: `crates/mnemosyne/tests/gbrain_backend_contract.rs`

- [ ] Implement the enriched `MemoryService` contract or an internal supplemental-service trait with identical record/recall semantics.
- [ ] `record` validates and enqueues locally; it does not wait for GBrain network success.
- [ ] Worker drains bounded batches, honors cancellation/shutdown, persists retry state, and resumes on startup. Permanent malformed entries become inspectable dead letters.
- [ ] `recall` has a strict latency budget; unavailable/slow GBrain returns an empty supplemental set plus health evidence, not an error that aborts a Goal.
- [ ] `forget` requires a supported remote deletion/tombstone contract. Until verified, return explicit Unsupported and never pretend deletion succeeded.
- [ ] Run the same contract suite against local and GBrain implementations, including provenance/temporal fields and outage behavior.
- [ ] Run `cargo test -p mnemosyne --test gbrain_backend_contract`; expect PASS.
- [ ] Commit `feat(mnemosyne): implement optional GBrain backend`.

## 7. Task 6 — Compose local and GBrain memory services

**Files:**

- Create: `crates/mnemosyne/src/composite_service.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/gbrain_bootstrap.rs`

- [ ] `CompositeMemoryService` always records to the local service first, then enqueues only policy-selected records to GBrain.
- [ ] Selection initially includes approved architecture decisions and terminal Goal summaries; raw chat, credentials, sensitive email bodies, transient tool output, and Dasein/self-memory are excluded.
- [ ] Recall queries local and GBrain within independent budgets, normalizes/merges by stable provenance key, prefers the newest valid version, and retains superseded entries when explicitly requested.
- [ ] Bootstrap constructs local-only when disabled/misconfigured and composite when enabled; GBrain startup failure marks degraded health but daemon initialization succeeds.
- [ ] Shutdown stops intake, persists outstanding local queue work, and bounds worker join time.
- [ ] Test disabled, healthy, unavailable-at-start, outage-after-start, slow recall, duplicate local/remote fact, malformed remote item, and shutdown with queued entries.
- [ ] Run `cargo test -p executive --test gbrain_bootstrap`; expect PASS.
- [ ] Commit `feat(executive): compose optional GBrain memory`.

## 8. Task 7 — Record decisions and Goal outcomes

**Files:**

- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Modify: `crates/executive/src/impl/goal/verification.rs`
- Create: `crates/executive/src/impl/memory_projection.rs`
- Test: `crates/executive/tests/goal_memory_projection.rs`

- [ ] Project only persisted, terminal/approved Goal summaries and explicitly approved architecture decisions.
- [ ] Include Goal/attempt/artifact/approval IDs, source commit, outcome, verification evidence, created/valid times, principal/sensitivity, and supersedes relation.
- [ ] Use deterministic record IDs so restart/replayed completion events cannot duplicate GBrain entries.
- [ ] A newer architecture decision references the old decision as superseded; both remain recallable and current-only recall returns the valid new one.
- [ ] Projection failure/degraded queue never changes the Goal's terminal result; record a health/audit event instead.
- [ ] Test successful/failed/rejected Goals, duplicate terminal events, revised decision, stale decision query, sensitive exclusions, and GBrain outage.
- [ ] Run `cargo test -p executive --test goal_memory_projection`; expect PASS.
- [ ] Commit `feat(executive): project durable outcomes to memory`.

## 9. Task 8 — Bound recall projection into Agora

**Files:**

- Modify: `crates/executive/src/service/daemon_turn/injection.rs`
- Modify: `crates/executive/src/impl/hook_lifecycle/recall_inject.rs`
- Test: `crates/executive/tests/gbrain_recall_injection.rs`

- [ ] Convert recall items into a bounded context section labeled with source, observed time, valid interval, current/superseded state, and confidence.
- [ ] Default query requests Current entries only; a historical query may include superseded/expired results explicitly.
- [ ] Enforce item, byte, and latency budgets and deterministic ranking; untrusted remote text is data, not instructions.
- [ ] Never inject credentials, high-sensitivity excluded records, Dasein mutations, or remote tool directives.
- [ ] Test prompt-injection-shaped memory, stale/current conflicts, provenance rendering, byte truncation, timeout, empty supplemental recall, and local fallback.
- [ ] Run `cargo test -p executive --test gbrain_recall_injection`; expect PASS.
- [ ] Commit `feat(executive): inject bounded GBrain recall`.

## 10. Task 9 — Add local GBrain deployment assets

**Files:**

- Create: `deploy/gbrain/compose.yaml`
- Create: `deploy/gbrain/.env.example`
- Create: `deploy/gbrain/README.md`
- Modify: `crates/executive/src/core/config/mod.rs`
- Modify: `crates/executive/src/core/config/infra.rs`
- Modify: `config/default.toml`

- [ ] Pin GBrain image by immutable digest and document the exact API/schema version used by DTO fixtures.
- [ ] Bind service ports to loopback or an internal container network only; configure persistent volumes, healthcheck, restart policy, resource limits, and log rotation.
- [ ] Keep real credentials outside git and reference secret-file mounts; `.env.example` contains placeholders only.
- [ ] Document initialize, upgrade, backup, restore, health, dead-letter inspection, and full disable/fallback procedures.
- [ ] Validate `docker compose config`; start against a disposable volume and run client contract smoke tests.
- [ ] Commit `docs(deploy): add local GBrain service assets`.

## 11. M8 release audit

- [ ] Run formatting, all scoped tests, workspace tests, and workspace build.
- [ ] Prove GBrain down at startup and mid-Goal does not block Telegram, Goal execution, approval, or local recall.
- [ ] Kill daemon after enqueue and GBrain after ingest-before-ack; restart both and prove one logical remote record with no silent drop.
- [ ] Exhaust retry for malformed 4xx and prove inspectable dead-letter state without retry storm.
- [ ] Record two architecture decisions with a supersedes edge and prove current-only and historical recall differ correctly.
- [ ] Audit GBrain requests, spool, logs, and injected context for tokens, raw sensitive mail, and Dasein records.
- [ ] Restore GBrain volume/spool from backup and verify ingestion/recall consistency.

## 12. DeepSeek batches

1. Tasks 1–3: contract, DTO/config, REST client.
2. Tasks 4–5: spool, backend, lifecycle.
3. Tasks 6–8: composition, projection, recall injection.
4. Tasks 9–11: deployment and failure audit.

Guardrails:

```text
Do not replace DefaultMemoryService.
Do not let GBrain outage fail core Goal execution.
Do not acknowledge ingestion before local spool commit.
Do not flatten provenance or temporal validity into plain text.
Do not send Dasein/self-memory, credentials, or sensitive raw mail.
Do not guess an undocumented GBrain API contract.
Stop after each batch with outage/restart evidence.
```
