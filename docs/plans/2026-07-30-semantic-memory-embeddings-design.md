# Semantic Memory — Real Embeddings + Vector Store + Hybrid Ranking (A)

**Date:** 2026-07-30
**Status:** Revised design; provider, store, async lifecycle, fusion, and
degradation decisions locked; implementation plan pending
**Scope:** Upgrade memory recall from keyword-only (FTS5) plus a deterministic
32-dim hash pseudo-embedding to **real semantic retrieval** — a pluggable LLM
embedding provider and a real vector store — while keeping FTS5 as the lexical
arm of a hybrid ranker. This is Workstream A of the Wave 2 retrieval upgrade.

> Roadmap context: the recall *pipeline* is already hybrid-shaped and wired in
> code (implemented and tested; NOT asserted as production-accepted — see the
> acceptance caveat in the E′ doc §7 — and the vector arm is in fact off in every
> shipped runtime, see §1). `hybrid_recall_with_metrics`
> (`crates/mnemosyne/src/recall/pipeline.rs:345-455`) already runs a lexical arm
> and an optional vector arm behind a governed `ScopePredicate`, with fail-open
> degradation (`DegradedSource`, `pipeline.rs:236-255`), MMR, and autocut. The
> service composition switch `DefaultMemoryService::with_memory_hybrid`
> (`crates/mnemosyne/src/service.rs:479-485`) and the vector-backend install seam
> `with_vector_search_backend` (`service.rs:487-501`) already exist, gated by an
> endpoint-pinned `EmbeddingCredentialGrant` (`crates/mnemosyne/src/credential.rs:41-100`).
> **What is missing is not the plumbing — it is a real embedding model, a real
> persistent vector index, and a scale-free fusion.** A is therefore an
> infill of three named gaps, not a rewrite. The Cargo feature placeholders
> `vector-lance` / `vector-qdrant` (`crates/mnemosyne/Cargo.toml:40-41`, mirrored
> in the workspace cfg lint at `Cargo.toml:68`) confirm the repo already
> anticipated pluggable vector backends.

## 1. Background & Problem

Recall today has two arms, and neither is semantically meaningful.

**Lexical arm (keep).** SemanticMemory persists content into an FTS5 virtual
table (`semantic_fts USING fts5(title, content, ... tokenize='porter unicode61')`,
`crates/mnemosyne/src/backends/semantic/schema.rs:268-271`); EpisodicMemory has
the equivalent FTS5 backend (`crates/mnemosyne/src/backends/episodic/mod.rs`).
The production recall path assembles lexical candidates into a
`LexicalSnapshotBackend` (`crates/mnemosyne/src/service.rs:422-449`), which scores
purely by inverse rank (`score = 1.0 / (index + 1)`, `service.rs:443`).

**Vector arm (inadequate).** The only embedding provider is
`HashEmbeddingProvider` (`schema.rs:118-137`), a 32-dim deterministic byte hash
(`hash_embedding`, `schema.rs:144-158`). Its own doc comment states the vectors
"are NOT semantically meaningful … they exist solely to exercise the VectorIndex
plumbing" (`schema.rs:113-117`). The index it feeds is `VectorIndex`
(`schema.rs:29-85`), an in-memory brute-force cosine scan
(`cosine_similarity`, `schema.rs:94-105`) that is never persisted and is rebuilt
empty on every process start. Embeddings are generated synchronously on write
(`SemanticMemory::store` → `generate_embedding` → `vector_index.upsert`,
`crates/mnemosyne/src/backends/semantic/storage.rs:18-80`).

Current recall flow (verified):

```
turn -> ContextAssembler::load(request)                 crates/executive/src/application/context_assembler.rs:132
     -> MemoryService::recall(RecallRequest{...})       context_assembler.rs:134-148 (bounded: max_items, max_bytes, timeout, fail-open)
     -> DefaultMemoryService::recall(req)               crates/mnemosyne/src/service.rs:928
     -> recall_with_prefilter -> lexical candidates     service.rs:937 (RecallPreFilter scope gate)
     -> hybrid_recall_with_metrics(fts, vector, ...)    service.rs:945-956  ->  crates/mnemosyne/src/recall/pipeline.rs:345
        fts arm:    LexicalSnapshotBackend (FTS5)        pipeline.rs:358-366
        vector arm: self.vector_search (Option)          pipeline.rs:368-384  (None in every shipped runtime)
     -> merge: concat both arms, sort by RAW score       pipeline.rs:387-407   <-- SCALE-MIXING BUG
     -> MMR (optional) -> autocut -> byte budget          pipeline.rs:409-451
```

Three concrete inadequacies:

1. **No semantic signal.** A query for "how do I cap child agent tools" cannot
   retrieve a note titled "capability attenuation" — the hash embedding has no
   notion of meaning, and FTS5 needs a shared surface token. Vector recall is off
   in every shipped runtime (`vector_search: None` default at `service.rs:469`;
   only `token_max` even requests it, `pipeline.rs:216`).
2. **Scale-mixing fusion.** Even with a real vector arm, the merge at
   `pipeline.rs:395-407` sorts the *union* of both arms by each candidate's raw
   `score`. The FTS arm emits reciprocal-rank scores in `(0,1]`
   (`service.rs:443`) while a vector arm emits cosine similarity in `[-1,1]`.
   Concatenating and sorting by raw score lets one arm's scale dominate the
   other; there is no rank normalization.
3. **Ephemeral, unscalable index.** `VectorIndex` is RAM-only and O(n) per query
   (`schema.rs:61-79`); it is lost on restart and never populated for the
   production `DefaultMemoryService` recall path (that path expects an installed
   `RecallSearchBackend`, not the SemanticMemory-internal index).

The contract for a real vector arm already exists: any vector backend implements
`RecallSearchBackend` (`pipeline.rs:272-280`) and **must apply the
`ScopePredicate` before materializing candidates** (enforced by the
defense-in-depth test at `pipeline.rs:668-705`). So A must fill the model + store
+ fusion gaps *without* touching the governance boundary.

## 2. Goals / Non-goals

**Goals**

1. A real, pluggable embedding provider behind the existing `EmbeddingProvider`
   trait (`crates/fabric/src/include/memory.rs:127-134`), mirroring the
   `LlmProvider` provider family (`crates/fabric/src/types/llm_types.rs:76-114`):
   Ollama (local) and OpenAI-compatible, plus a **deterministic
   offline test provider** so CI needs no network or key.
2. A real, persistent vector store implementing `RecallSearchBackend`
   (`pipeline.rs:272-280`), scope-gated identically to the lexical arm.
3. Scale-free hybrid fusion (Reciprocal Rank Fusion) replacing the raw-score
   concat at `pipeline.rs:395-407`.
4. A migration off the 32-dim hash index, including a dimension/model change and
   a backfill of existing records.
5. Preserve, unchanged, the three invariants A must not regress: scope-gating
   (`RecallPreFilter` / `ScopePredicate`, `pipeline.rs:19-97`), fail-open
   degradation (`DegradedSource`, `pipeline.rs:236-255`), and the pre-turn
   recall budget (items / bytes / timeout, `context_assembler.rs:132-158`).

**Non-goals**

- **Knowledge graph, multi-hop traversal, and synthesis.** Typed-edge extraction
  and `think`-style cited synthesis are gbrain's job (see §7); A is embeddings +
  vector search + hybrid ranking only. The local `knowledge_graph` field
  (`service.rs:408`) and `llm-synthesis` feature (`Cargo.toml:43`) are untouched.
- **Cross-encoder reranking.** gbrain's `zerank-2` rerank stage is out of scope;
  A stops at RRF fusion + the existing MMR/autocut. A rerank seam is noted as a
  future extension, not built.
- **Query expansion / intent classification** (gbrain's pre-retrieval layer).
  The `expansion_enabled` knob already exists in the mode bundles
  (`pipeline.rs:169`) but wiring an expander is out of scope for A.
- **New governance semantics.** A reuses `ScopePredicate`, `EmbeddingCredentialGrant`,
  and `DegradedSource` verbatim; it introduces no new trust boundary.
- **Changing the pre-turn recall budget contract** at `context_assembler.rs:132-158`.

## 3. Design

### 3.1 Embedding provider abstraction

**The trait already exists** and is the right shape:
`EmbeddingProvider { async fn embed(&self, text) -> Result<Vec<f32>>; fn dimension(&self) -> usize; }`
(`crates/fabric/src/include/memory.rs:127-134`).

**Locked placement:** concrete adapters live under
`crates/mnemosyne/src/adapters/embedding/`. The endpoint-pinned
`EmbeddingCredentialGrant` already lives in Mnemosyne
(`crates/mnemosyne/src/credential.rs:41-109`); placing adapters in Cognit would
either create the wrong dependency direction or duplicate that authority. Share
only dependency-neutral HTTP/error helpers when they can be extracted cleanly;
do not move secret authorization into a generic transport helper.

Concrete implementations:

- `OpenAiEmbeddingProvider` — POSTs `/v1/embeddings`; the OpenAI-compatible
    surface also covers Azure, LiteLLM, and most gateways (the same shape gbrain
    treats as its default fallback). Default model `text-embedding-3-small`
    (1536-dim) or `-3-large` truncated to 1536 via the `dimensions` parameter.
- `OllamaEmbeddingProvider` — local `/api/embeddings`, model
    `nomic-embed-text` (768-dim). Native-first, key-free, offline-capable.
- A future Voyage adapter may use the same provider contract, but is not in
    the initial implementation scope.
- `DeterministicTestEmbeddingProvider` — a real, seeded, higher-dim descendant
    of today's `HashEmbeddingProvider` used **only** in tests. `HashEmbeddingProvider`
    (`schema.rs:118-158`) is retained and repurposed as this test double (renamed
    in-place is optional); it is removed from any production composition path.

The service remains model-agnostic by programming against the Fabric trait. The
existing `search_by_embedding` and `generate_embedding` (`schema.rs:206-247`)
already use `Arc<dyn EmbeddingProvider>`, so no consumer signature churn is
required.

Trait extension (additive, default-provided): add
`async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>` with a
default that loops `embed`, plus `fn model_id(&self) -> &str`. Batch is required
for an efficient backfill (§3.4) and for consolidation-time embedding (§3.5);
`model_id` is required for the index-provenance stamp (§3.4).

**Provider selection** is config-driven (a new `[memory.embedding]` section:
`enabled`, `provider`, `model`, `base_url`, `dimensions`). It is disabled by
default; enabling it requires an explicit provider/model. Ollama is the
native-first example, not an automatic network call. Executive bootstrap, which
currently constructs `DefaultMemoryService` at
`crates/executive/src/host/daemon/bootstrap/request.rs:692-703`, constructs the
Mnemosyne provider adapter and index ports and injects them into the service.

Endpoint authorization is checked on every remote batch/query, not once at
bootstrap. The Mnemosyne provider adapter holds the endpoint-pinned
`EmbeddingCredentialGrant`
and a `Clock`, calls `secret_if_approved(base_url, now)` immediately before each
request (`credential.rs:97-109`), rejects cross-origin redirects, and surfaces
expiry/rotation as typed degradation. The existing bootstrap-time boolean in
`with_vector_search_backend` (`service.rs:487-500`) must therefore be replaced by
a per-call authorization result carried by the vector reader.

Remote calls also require a machine/provider-scoped permit keyed by canonical
endpoint and model. A shared `ProviderBackpressurePort` owns concurrency and
cooldown across sessions and daemon callers; workers honor provider
`Retry-After` advice through that coordinator. Per-job retry counters are audit
state only and are never presented as a substitute for cross-session
coordination. If the coordinator is unavailable, remote embedding degrades to
FTS-only rather than making an ungoverned request.

### 3.2 Vector store and read/write contracts

The recall path needs a *persistent* `RecallSearchBackend` (`pipeline.rs:272-280`),
not the RAM-only `VectorIndex`. Options, judged for a native-first, always-on,
single-binary daemon:

| Option | Deploy | Persistence | ANN | Verdict |
|---|---|---|---|---|
| **B1 first-party SQLite exact-vector backend** | zero extra service; `rusqlite` is already bundled (`crates/mnemosyne/Cargo.toml:18`) | single DB file, same backup unit as FTS5 | exact cosine scan; bounded personal-memory baseline | **Selected default** |
| B1b sqlite-vec extension | in-process extension with target-specific packaging | single DB file | extension-defined KNN | optional optimization after target validation |
| B2 qdrant | separate daemon/sidecar or embedded lib | own storage | HNSW, mature | scale-out; already reserved as `vector-qdrant` (`Cargo.toml:41`) |
| B3 lance | in-process columnar files | Lance dataset dir | IVF/HNSW | large corpora / columnar; reserved as `vector-lance` (`Cargo.toml:40`) |

**Locked decision: B1 first-party SQLite exact-vector backend as the initial
default**, with sqlite-vec/qdrant/lance optional after target-specific packaging
and performance acceptance. This avoids making an unverified native extension a
system deployment prerequisite while still providing real model vectors and
durable storage. Reasoning:

- It matches the "native-first, zero-config" posture — no second process to
  supervise, mirroring gbrain's own default of in-process PGLite for personal
  brains with Postgres/pgvector reserved for scale (gbrain `README.md:282`).
- It reuses the bundled `rusqlite` dependency. Vector rows and embedding jobs
  share one dedicated database/backup unit; consolidation source records remain
  authoritative and are not coupled to a network embedding call in one SQL
  transaction.
- Scope metadata (the `scope_keys` a `ScopePredicate` binds, `pipeline.rs:73-77`)
  becomes ordinary SQL columns filtered *before* the KNN, satisfying the
  "apply predicate before materializing candidates" contract naturally
  (`pipeline.rs:271-280`).

The existing `RecallSearchBackend` is read-only (`pipeline.rs:271-280`) and
cannot support consolidation/backfill by itself. Add a separate write/lifecycle
port rather than downcasting the reader:

```rust
#[async_trait]
pub trait VectorIndexWriter: Send + Sync {
    async fn upsert_batch(&self, records: &[EmbeddedRecord]) -> anyhow::Result<()>;
    async fn remove(&self, record_ids: &[MemoryRecordId]) -> anyhow::Result<()>;
    async fn state(&self) -> anyhow::Result<VectorIndexState>;
}

pub struct VectorIndexPorts {
    pub reader: Arc<dyn RecallSearchBackend>,
    pub writer: Arc<dyn VectorIndexWriter>,
}
```

Concrete type: `SqliteVectorBackend`, implementing both ports over one database.
The reader is installed through `with_vector_search_backend`; the writer is
injected into a dedicated embedding worker. Optional qdrant/lance backends
implement the same two ports.

### 3.3 Hybrid ranking (fusion)

Replace the raw-score concat/sort at `pipeline.rs:395-407` with **Reciprocal
Rank Fusion (RRF)**, the same rank-based, scale-free fusion gbrain uses
("each strategy votes", gbrain `RETRIEVAL.md:9`).

- Options: (i) weighted linear blend of normalized scores — rejected, requires a
  tuned per-arm weight and per-arm score normalization that is brittle across
  models; (ii) **RRF** — recommended, needs no score calibration, only ranks.
- Mechanism: for each arm, rank its candidates; each candidate accrues
  `Σ_arms 1 / (k + rank_arm)` with a fixed `k = 60` (standard). The fused score
  replaces the raw `score` used by the existing sort, MMR
  (`deterministic_mmr`, `pipeline.rs:460-493`), autocut (`pipeline.rs:414-431`),
  and evidence stamping (`pipeline.rs:433-451`) — all of which stay as-is.
- Determinism is preserved: RRF is a pure function of the two arms' rankings, and
  the existing tie-break on `record_id` (`pipeline.rs:400-406`) keeps the total
  order stable, so the repeatability proptest (`pipeline.rs:798-864`) still holds
  after re-baselining expected values.
- Governance is unchanged: RRF runs **after** `predicate.allows` retain
  (`pipeline.rs:388`), so no candidate can enter fusion across a scope boundary.

Implementation must keep lexical and vector results in separate ranked vectors
until fusion. The current single `ranked` vector loses arm provenance before
sort/dedup (`pipeline.rs:355-407`) and cannot calculate RRF votes correctly.
Deduplicate by `record_id` only while accumulating per-arm ranks, then emit one
fused candidate.

Fusion lives inside `hybrid_recall_with_metrics`; no new merge entry point. The
mode bundles keep their meaning — `conservative`/`balanced` stay FTS-first
(`pipeline.rs:178-208`), `token_max` turns the vector arm on
(`pipeline.rs:210-224`); RRF simply fuses whichever arms produced candidates.

### 3.4 Migration / backfill and the dimension change

Today's index is 32-dim hash (`HashEmbeddingProvider::new(32)` in tests,
`schema.rs:34,118`); a real provider is 768/1024/1536-dim (§3.1). Dimension and
model are both changing, so the old vectors are meaningless and must be discarded,
not reinterpreted.

- **Provenance stamp.** The vector store records `(embedding_dim, model_id,
  provider_id, rotation_generation)` per row and once at index level. `model_id`
  comes from the §3.1 trait extension; `rotation_generation` already exists on the
  grant (`credential.rs:48`).
- **Mismatch = stale, not error.** On startup, if the configured provider's
  `(dimension, model_id)` differs from the stored index stamp, the backend reports
  `index_stale` in its `SearchOutcome` (`pipeline.rs:263-269`), which surfaces as
  `DegradedSource::VectorIndexStale` (`pipeline.rs:376-378`) and lets recall serve
  the last-valid snapshot (`LastValidSnapshotBackend`, `pipeline.rs:285-319`)
  and/or fall back to FTS while a backfill runs. No dimension-mismatch panic.
- **Backfill job.** A durable backfill queue iterates current records, calls
  `embed_batch` (§3.1), and upserts into the new-dimension index, then flips the
  index stamp to non-stale. Jobs are idempotent by
  `(record_id, provider_id, model_id, dimension, rotation_generation)` and
  resumable by durable job state/watermark, so a crash reclaims pending work.
- **No dual-dimension index.** Because old vectors are 32-dim noise, the migration
  drops them rather than maintaining two indexes; the FTS arm covers recall during
  the backfill window, which is exactly the fail-open path A already has.

### 3.5 Where embedding generation happens on the write path

Two write moments; recommendation differs per moment because their latency
budgets differ.

- **Query embedding — synchronous, budgeted.** The recall path must embed the
  query text to run KNN. This happens inside the vector `RecallSearchBackend`
  call, which is already wrapped by the pre-turn `tokio::time::timeout`
  (`context_assembler.rs:144-149`, `recall_timeout_ms`) and fails open to FTS on
  timeout via `DegradedSource::EmbeddingTimeout` (`pipeline.rs:381`). No new
  budget needed — the query embed inherits the existing turn recall timeout.
- **Content embedding — asynchronous, after consolidation.** Do **not** block a
  turn to embed stored content. Today `SemanticMemory::store` embeds inline
  (`storage.rs:24`); for the production recall index we instead embed during
  consolidation, where records are already being written
  (`ScopedConsolidator::run` → `commit_decisions`,
  `crates/mnemosyne/src/consolidation/consolidator.rs:34-99,84`). Options:
  - Option E1 — synchronous embed on every episodic write. Rejected: puts a
    network round-trip on the hot turn path.
  - **Option E2 (selected) — enqueue after consolidation.** `ScopedConsolidator`
    is synchronous today (`consolidator.rs:34-99`), while `embed_batch` is async;
    it must not await a provider while holding the scope lease. After
    `commit_decisions`, transactionally enqueue durable embedding jobs for new
    `Insert`/`Supersede` record IDs. A separate bounded async
    `MemoryEmbeddingWorker` loads the authoritative record, embeds it, writes via
    `VectorIndexWriter`, and marks the job terminal. `Merge`/`Reject` enqueue
    nothing; superseded/deleted records enqueue removal jobs.
  - The small SemanticMemory-internal path (`storage.rs:18-80`) may keep its
    inline embed for its own `search_by_embedding` API, but it is not the
    production recall index and is not on the turn path.

### 3.6 Preserving scope-gating, fail-open, and budget

These are invariants, restated as acceptance constraints for the new code:

- **Scope-gating.** `SqliteVectorBackend::search` binds the `ScopePredicate`
  `scope_keys` / `allowed_authorities` / `max_sensitivity_ord`
  (`pipeline.rs:73-77`) as SQL `WHERE` filters *before* the KNN, exactly as the
  trait doc mandates (`pipeline.rs:271-280`); the merge-boundary
  `predicate.allows` retain (`pipeline.rs:388`) remains as defense in depth.
- **Fail-open.** Provider down / timeout / untrusted endpoint all map to existing
  `DegradedSource` variants (`pipeline.rs:236-255`); the vector arm never
  suppresses the lexical arm (`hybrid_recall` doc, `pipeline.rs:329-331`), and an
  untrusted endpoint is never even called (`pipeline.rs:369-373`, credential gate
  at `service.rs:499`).
- **Budget.** Pre-turn recall stays bounded to `recall_max_items` /
  `recall_max_bytes` / `recall_timeout_ms` with empty-on-failure
  (`context_assembler.rs:135-158`); the byte cap in fusion (`pipeline.rs:434-451`)
  is unchanged.

### 3.7 Data-flow diagram

```
WRITE PATH (async, off the turn)
  episodic event ──▶ consolidation candidates
     └─▶ ScopedConsolidator::run                consolidator.rs:34
          └─▶ commit_decisions (Insert/Supersede) consolidator.rs:84,138
               └─▶ durable embedding job enqueue  [NEW]
                    └─▶ MemoryEmbeddingWorker
                         └─▶ EmbeddingProvider::embed_batch  [NEW, §3.1]
                              └─▶ SqliteVectorBackend.upsert_batch  [NEW, §3.2/§3.4]

READ PATH (sync, budgeted, fail-open)
  turn ─▶ ContextAssembler::load                 context_assembler.rs:132
       └─▶ MemoryService::recall (items/bytes/timeout)  context_assembler.rs:134-149
            └─▶ DefaultMemoryService::recall       service.rs:928
                 ├─ lexical arm: FTS5 ▶ LexicalSnapshotBackend   service.rs:942 (FTS5 schema.rs:268)
                 └─ vector arm:  query embed ▶ SqliteVectorBackend.search(ScopePredicate)  [NEW]
                                   (embed via EmbeddingProvider; scope-filter BEFORE KNN)  pipeline.rs:271-280
            └─▶ hybrid_recall_with_metrics          pipeline.rs:345
                 ├─ predicate.allows retain (govern) pipeline.rs:388
                 ├─ RRF fuse  [NEW, §3.3 — replaces raw-score sort pipeline.rs:395-407]
                 ├─ MMR (optional)                   pipeline.rs:409,460
                 ├─ autocut (optional)               pipeline.rs:414
                 └─ byte-budget + evidence stamp     pipeline.rs:434-451
            └─▶ DegradedSource on any vector failure ▶ FTS-only  pipeline.rs:236-255,368-384
```

## 4. Error handling

- **Embedding provider down / 5xx / timeout.** Do not reuse
  `EmbeddingTimeout` for different semantics. Extend `DegradedSource` with
  `EmbeddingProviderUnavailable`, keep `EmbeddingTimeout` for an actual deadline,
  and add `VectorStoreError` for local index I/O. All three proceed FTS-only; the
  turn still does not fail (`context_assembler.rs:150-152`).
- **Untrusted / rotated endpoint.** `EmbeddingCredentialGrant::approved_for`
  false (`credential.rs:89-95`) → `embedding_endpoint_trusted = false`
  (`service.rs:499`) → vector arm is never invoked
  (`pipeline.rs:369-373`), `DegradedSource::EmbeddingEndpointUntrusted` recorded
  (`pipeline.rs:371`), `embedding_credential_rejected` metric incremented
  (`service.rs:972`). No secret can cross an origin change.
- **Dimension / model mismatch.** Not a panic. Detected by the index stamp
  (§3.4) → `index_stale` → `VectorIndexStale` → last-valid snapshot + FTS
  (`pipeline.rs:285-319,376-378`) until backfill completes.
- **Backfill failure.** Idempotent + watermark-resumable (§3.4); a partial index
  is simply an FTS-heavier recall until it finishes. No data loss (source records
  are the consolidation store, unchanged).
- **Vector store I/O error at query time.** `SqliteVectorBackend::search` returns
  a typed `VectorStoreError`; FTS arm
  unaffected. Lexical FTS failure remains independently reported
  (`FtsDbError`, `pipeline.rs:364`), never synthesized into a fake success
  (proven by `fts_failure_is_degraded_without_synthetic_success`,
  `pipeline.rs:645-665`).
- **Latency budget.** Query embed + KNN run under the existing
  `recall_timeout_ms` wrapper (`context_assembler.rs:144-149`). The HTTP client
  must also use a shorter bounded connect/request timeout so cancellation is
  prompt; the outer recall deadline remains authoritative.

## 5. Verification

Deterministic, offline-first — no network or key in CI.

Unit tests (`crates/mnemosyne`):

- `DeterministicTestEmbeddingProvider` (§3.1): same text → same vector, correct
  `dimension()`, `embed_batch` == mapped `embed` (extends the existing
  `test_hash_embedding_deterministic` / `_different_texts`,
  `crates/mnemosyne/src/backends/semantic/mod.rs:216-231`).
- `SqliteVectorBackend`: upsert/search/remove round-trips and **persistence across
  reopen** (the RAM-only gap today); top-k ordering (adapts the `VectorIndex`
  tests `mod.rs:149-213`); scope filter applied *before* KNN — a denied-scope
  row is never returned (mirrors `vector_candidates_cannot_cross_scope_predicate`,
  `pipeline.rs:668-705`).
- RRF fusion (§3.3): given known per-arm ranks, the fused order is the RRF order;
  fusion is scale-invariant (FTS reciprocal-rank vs cosine no longer lets one arm
  dominate); determinism/repeatability proptest still passes after re-baselining
  (`pipeline.rs:798-864`).

Recall-quality tests:

- A semantic hit that FTS cannot make (query and document share no surface token
  but are paraphrases) is retrieved by the vector arm and ranked by RRF — the
  regression the hash provider could never satisfy.
- Fail-open: with the vector backend erroring/untrusted, results equal FTS-only
  and the right `DegradedSource` is reported (extends
  `unavailable_embedding_falls_back_to_fts` / `untrusted_endpoint_never_invokes_vector`,
  `pipeline.rs:591-643`).
- Per-call security: an expired/rotated grant, cross-origin redirect, or missing
  machine/provider permit makes zero HTTP calls and reports the specific degraded
  source.
- Backpressure: two session workers targeting the same endpoint/model share one
  concurrency/cooldown record; a provider `Retry-After` delays both, while a
  different provider key remains independent.
- Observability reports embedding provider requests, retries, queue attempts,
  vector queries, lexical queries, latency, and degraded outcomes as separate
  counters. It never infers provider calls from job attempts or tool calls.

Migration test:

- Seed a 32-dim (old) index stamp, start with a 1536-dim provider → backend
  reports `VectorIndexStale`; run backfill → stamp flips, vectors present, recall
  returns semantic hits; backfill is idempotent on re-run.

Commands (via the wrapper, narrowest first):

```
bash scripts/cargo-agent.sh test -p fabric --lib memory
bash scripts/cargo-agent.sh test -p mnemosyne
bash scripts/cargo-agent.sh test -p executive memory
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

After implementation, the integration owner must run
`sudo bash scripts/aletheon.sh deploy`, prove equal SHA-256 digests for the
release, installed, machine-daemon, and user-daemon executables, observe stable
restart counters, and complete a real LLM-backed `/usr/bin/aletheon` request through the
official user socket with explicitly configured embeddings. The persisted
embedding job, vector row/provenance, recall result, rendered response, and daemon
logs must agree. A temporary home, alternate socket, direct provider call, or
isolated daemon is diagnostic evidence only.

## 6. Files touched

- `crates/fabric/src/include/memory.rs` — extend `EmbeddingProvider` with default
  `embed_batch` + `model_id` (additive; trait at `:127-134`).
- `crates/mnemosyne/src/adapters/embedding/` — new
  `OpenAiEmbeddingProvider` and `OllamaEmbeddingProvider` adapters. They own the
  existing Mnemosyne credential grant, per-call endpoint validation, bounded
  HTTP timeouts, redirect rejection, and provider error classification.
- `crates/mnemosyne/src/backends/semantic/schema.rs` — keep
  `HashEmbeddingProvider` as `DeterministicTestEmbeddingProvider` (test-only);
  the RAM `VectorIndex` stays for the SemanticMemory-internal API but is no longer
  the production recall index.
- `crates/mnemosyne/src/backends/vector/` (new) — `VectorIndexWriter`, lifecycle
  types, and `SqliteVectorBackend` implementing both read and write ports, with
  index stamp + scope-filtered exact KNN.
- `crates/mnemosyne/src/recall/pipeline.rs` — replace raw-score fusion
  (`:395-407`) with RRF; no change to governance, MMR, autocut, or budget.
- `crates/mnemosyne/src/consolidation/consolidator.rs` — after
  `commit_decisions` (`:84`), enqueue durable embedding/removal jobs; never await
  a provider while holding the consolidation lease.
- `crates/mnemosyne/src/embedding_worker.rs` (new) — bounded async job worker,
  retry/cooldown, backfill watermark, terminal receipts, and shutdown draining.
- `crates/mnemosyne/src/service.rs` — accept/inject provider, vector reader, and
  job queue; construction and configuration remain in Executive bootstrap.
- `crates/executive/src/host/daemon/bootstrap/request.rs` — construct the
  configured Mnemosyne provider and index ports, inject the machine/provider
  backpressure port and per-call grant, install the reader via
  `with_vector_search_backend`, and supervise the worker.
- `crates/mnemosyne/Cargo.toml` — no new native extension in the initial slice;
  optional vector features gain dependencies only when separately implemented.
- Config: new `[memory.embedding]` section
  (`enabled/provider/model/base_url/dimensions/rotation_generation`),
  read at daemon bootstrap and paired with `EmbeddingCredentialGrant`
  (`credential.rs:66-84`).

## 7. Scope boundary

This is Workstream A of Wave 2 (retrieval upgrade). It delivers **local,
always-on semantic recall**: real embeddings, a persistent scope-gated vector
store, and RRF hybrid fusion — nothing above ranking.

Relationship to gbrain (the optional supplemental backend,
`crates/executive/src/adapters/gbrain/mod.rs`,
`crates/mnemosyne/src/backends/supplemental/mod.rs`): gbrain owns the *higher*
layers — self-wiring knowledge graph, multi-hop graph-augmented retrieval, and
cited synthesis (gbrain `RETRIEVAL.md`, `README.md:255-292`). A does **not**
reimplement those. The clean split:

- **A (local, this doc):** first-party embeddings + vector KNN + RRF over the
  daemon's own episodic/semantic memory, on the hot recall path, offline-capable.
- **gbrain (supplemental, existing):** graph, traversal, rerank, and synthesis
  over a curated brain, reached through the supplemental transport
  (`backends/supplemental/mod.rs`), not the turn-critical recall path.

A explicitly excludes: knowledge-graph extraction, cross-encoder rerank, query
expansion/intent classification, and any change to the pre-turn recall budget or
the governance boundary. Those are either gbrain's job or a later wave.

## Locked review decisions

1. **Vector store default:** first-party SQLite exact-vector backend. Add an ANN
   backend only after corpus-size benchmarks and system deployment validation.
2. **gbrain-vs-local boundary:** A stays purely first-party
   embeddings + KNN + RRF and never delegates *semantics* to gbrain — i.e., gbrain
   remains a supplemental/synthesis layer, not the primary recall vector store.
   If instead gbrain should be the vector store, A shrinks to fusion + provider
   glue only.
3. **Provider default:** feature remains disabled until configured. Ollama is the
   native-first example; OpenAI-compatible is optional. Persisted provenance
   makes dimension/model changes trigger backfill rather than reinterpretation.
4. **Content cadence:** durable async jobs after consolidation; never inline on
   the turn path and never await the provider under the consolidation lease.
5. **Rerank:** A stops at RRF; cross-encoder reranking remains out of scope.
