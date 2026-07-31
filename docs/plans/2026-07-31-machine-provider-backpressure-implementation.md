# F1 Machine/Provider Backpressure Implementation Plan

**Design:** `docs/plans/2026-07-31-machine-provider-backpressure-design.md`

1. **Implemented:** add `ProviderBackpressureConfig` and deterministic schema.
2. **Implemented:** add provider-keyed shared state, fair permits, proactive
   request-start pacing, shared cooldown, queue deadline, and snapshots in
   `crates/cognit/src/adapters/inference/backpressure.rs`.
3. **Implemented:** wrap every canonical LLM transport and hold permits through
   stream termination (`crates/cognit/src/composition/inference_factory.rs:116-187`).
4. **Implemented in current worktree:** normalize LLM/embedding keys to endpoint
   plus model, and route remote embedding permit/cooldown operations through
   machine-core RPC (`crates/fabric/src/include/memory.rs:157-170`;
   `crates/executive/src/application/inference_port.rs:38-109`;
   `crates/executive/src/host/core_rpc/protocol.rs:13-102`).
5. **Implemented in current worktree:** export machine-core snapshots through user
   health without misreporting the user process's empty registry as machine state
   (`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-179`).
6. **Implemented:** add `min_request_interval_ms` to the typed provider policy,
   keep its compatibility default at zero, and configure the checked-in Leju
   default/system profiles at 15 seconds
   (`crates/cognit/src/config/mod.rs:631-652`; `config/default.toml:42-54`;
   `config/production.toml.example:9-19`). A no-advice transient failure now
   uses the shared 30-second bounded fallback
   (`crates/fabric/src/include/memory.rs:165-168`).
7. **Deterministic validation passed:** five backpressure tests, socket-backed
   permit retention/release, cooldown/snapshot RPC, schema snapshots, both
   checked-in configs, fallback cooldown, focused consumers, and
   `check -p aletheon`. Focused monitor, terminal snapshot, one-shot session,
   command classifier, and provider UTF-8 framing tests also pass.
8. **Installed evidence passed in part:** one full deploy and digest/restart
   gate passed; two concurrent independent official clients produced terminal
   answers and the machine snapshot observed `paced=1`, `queued=1`,
   `rejected=0`.
9. **Deployment gate accepted:** a later system deployment passed its
   official-client request, digest equality, and restart-stability gates. The
   current digest is intentionally read from typed runtime/deploy output rather
   than maintained as a mutable value in this plan. This does not erase the
   earlier failed-attempt evidence.
10. **Still pending:** child/embedding fan-out and three consecutive
   repository-analysis TUI runs with successful tools, structurally complete
   factually accurate answers, one authoritative terminal snapshot, and zero
   rendered/journal provider errors. The first three
   provider-clean runs were recomputed as PASS/FAIL/FAIL: run 2 ended with
   malformed incomplete Markdown; run 3 contained a failed `exec_command`.
   Later runs passed mechanical checks but still promoted historical plan facts
   into current risks; the three-run quality streak must restart after factual
   temporal grounding is corrected.

No LLM consumer may bypass the canonical provider factory. Remote semantic
embeddings introduced by A use their own transport adapter but must acquire the
machine-core coordinator rather than owning a second limiter.
