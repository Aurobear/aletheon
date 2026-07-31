# F1 Machine/Provider Backpressure Design

**Date:** 2026-07-31
**Status:** Implemented and deployed; installed pacing evidence and the latest
official-client smoke passed. Child/embedding fan-out and the three-run real-TUI
quality streak remain open.
**Requirement:** `docs/plans/2026-07-30-production-readiness-gap-analysis.md`
§3 item 1, §7 F1, §9

## Decision

Use the machine core as the single provider-admission authority. Every LLM provider
created by Cognit's canonical factory is wrapped by a provider-keyed coordinator in
that process (`crates/cognit/src/composition/inference_factory.rs:76-123`).
User-daemon remote embedding calls acquire and release the same authority over
authenticated core RPC; the socket lifetime is the permit lease
(`crates/executive/src/host/core_rpc/client.rs:20-24,72-106`;
`crates/executive/src/host/core_rpc/server.rs:200-214`). A Tokio semaphore supplies
FIFO admission. Provider `Retry-After` updates shared cooldown before another queued
caller may enter. The authority also spaces request starts by an optional typed
minimum interval, so a rolling-window quota is governed before the first 429
(`crates/cognit/src/adapters/inference/backpressure.rs:78-153`). A transient
`provider_unavailable` without advice receives a shared, bounded 30-second
fallback cooldown (`crates/fabric/src/include/memory.rs:165-168`;
`crates/cognit/src/composition/inference_factory.rs:131-143`). Non-streaming,
streaming, and embedding calls retain the permit until their authoritative
response/stream/request terminal or drop boundary.

Alternatives rejected:

1. Per-session retry only: cannot stop sibling role children from stampeding the same quota.
2. Scheduler-only coordination: misses direct routed providers and future embedding consumers.
3. A new coordination service beside the existing machine core: duplicates an
   already deployed authority. The current topology already splits
   `aletheon-core.service` from the user daemon, so process-local user-daemon state
   cannot satisfy machine scope.

The machine core is the authority in the current installed topology. User daemons
must not own independent provider state. Core RPC exposes only permit acquisition,
cooldown observation, and read-only snapshots; provider credentials and LLM
construction remain private to `RegistryInferencePort`.

## Contract

`ProviderConfig.backpressure` defines:

- `max_concurrent_requests` (default 2; zero is normalized to one),
- `min_request_interval_ms` (default 0 for compatibility; the checked-in Leju
  system/default profiles explicitly use 15 seconds),
- `queue_timeout_ms` (default 120 seconds),
- `max_cooldown_ms` (default 60 seconds).

The stable key is canonical endpoint plus model, built by
`fabric::memory::provider_backpressure_key`
(`crates/fabric/src/include/memory.rs:157-163`). LLM and embedding callers use that
same helper (`crates/cognit/src/composition/inference_factory.rs:118-121`;
`crates/mnemosyne/src/adapters/embedding.rs:54-57`). The first constructed policy
for a key is authoritative in the machine-core process. Queue timeout and
closed-coordinator errors fail closed with typed stable codes. Retry advice is
capped and monotonically extends, never shortens, an active cooldown.

Metrics remain separate from inference rounds, retries, tool calls, and context
occupancy: admitted, queued, paced, rejected, cooldown updates, active, and
available permits.

## Data flow

```text
main / role child / routed LLM caller
                  |
          authenticated core RPC
                  |
       machine core canonical factory --------+
                  |                            |
       provider-keyed coordinator              |
       cooldown -> FIFO permit -> start pacing |
                  |                            |
             LLM transport                     |
                                               |
user-daemon embedding -> core RPC permit lease-+
                  |                            |
          embedding HTTP request               |
                  +---- socket/request drop ---+
```

## Acceptance

1. Two independently resolved LLM instances with one endpoint/model key share one
   permit.
2. A provider-advised cooldown blocks the next caller and is capped/observable.
3. Queue deadline fails closed and increments rejection evidence.
4. A user-daemon embedding lease is held in machine core until its authenticated
   socket closes, including caller/process drop.
5. Health obtains machine-core snapshots over RPC and reports core metrics
   separately from any user-runtime state
   (`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-179`).
6. Canonical config schema is deterministic.
7. Installed acceptance observes child/embedding fan-out without provider request
   overlap beyond the configured limit and reports inference rounds, provider
   retries, cooldowns, and tool calls separately.

## 2026-07-31 installed evidence

- `sudo bash scripts/aletheon.sh deploy` passed once with digest
  `954b85a64e1bf6031886c3f6d51094c211ac670320ce5a6052fa307d4cd003ba`;
  release, `/usr/bin`, and both running daemon executables matched.
- Two concurrent `/usr/bin/aletheon --full` clients on the official user socket
  both returned their requested terminal text in 18.721 seconds. The
  machine-core snapshot then reported `admitted=4`, `paced=1`, `queued=1`,
  `rejected=0`, proving cross-session pacing rather than a per-session timer.
- This does **not** close the whole workstream. A later deployment gate failed:
  the upstream returned HTTP 503 without `Retry-After` on every attempt and the
  official client timed out. Backpressure correctly recorded cooldowns, but it
  cannot manufacture provider availability.
- A subsequent 16:40 system deployment passed the official-client request,
  digest equality, and two restart-stability windows. Its accepted digest is
  `38959ba3921f69b3e171935ff307e56ba7c2144522efe94b80245111c1f4b0c9`;
  the user daemon remained at `NRestarts=0`. The earlier 503 remains failure
  evidence for that attempt, not the latest installed state.
- A later real-TUI run was provider-clean and passed tool/snapshot/encoding
  assertions, but its maturity answer repeated a stale plan claim. It is not
  counted toward the final three-run streak because factual quality is part of
  repository-analysis acceptance.
