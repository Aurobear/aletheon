# Semantic Memory Embeddings Implementation Plan (A)

**Design:** `docs/plans/2026-07-30-semantic-memory-embeddings-design.md`

1. Extend the Fabric embedding contract with batch and model provenance; keep the deterministic provider test-only.
2. Replace raw-score concatenation with deterministic RRF inside the existing governed hybrid pipeline.
3. Add the first-party SQLite exact-vector reader/writer, SQL prefiltering, provenance stamps, persistence/reopen, stale-index, and removal tests.
4. Add endpoint-pinned OpenAI-compatible and Ollama adapters. Require the shared F1 permit before every remote request, disable redirects, and map endpoint/provider/timeout failures to distinct degradation signals.
5. Add default-off `[memory.embedding]` configuration and bootstrap the provider, vector backend, and bounded worker only when explicit provider/model/base URL/dimensions are present.
6. Add durable idempotent embedding jobs keyed by record/provider/model/dimension/rotation, reclaim expired leases, batch asynchronous embedding, and retry without blocking turns.
7. Validate offline Mnemosyne tests, deterministic config schema, Executive compilation, then installed recall/degradation/restart acceptance.
