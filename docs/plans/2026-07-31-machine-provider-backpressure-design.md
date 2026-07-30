# F1 Machine/Provider Backpressure Design

**Date:** 2026-07-31
**Status:** Approved recommendation, implemented pending installed acceptance
**Requirement:** `docs/plans/2026-07-30-production-readiness-gap-analysis.md:44,117-124,140-143,171-173`

## Decision

Use the canonical provider factory as the single admission choke point. Every provider created by Cognit is wrapped by a provider-keyed coordinator shared by all sessions and role children in the daemon. A Tokio semaphore supplies FIFO admission; a provider `Retry-After` updates shared cooldown state before another queued caller may enter. Both non-streaming and streaming calls retain their permit until the authoritative response or stream terminal/drop boundary.

Alternatives rejected:

1. Per-session retry only: cannot stop sibling role children from stampeding the same quota.
2. Scheduler-only coordination: misses direct routed providers and future embedding consumers.
3. A new remote coordination service: adds availability and deployment complexity without evidence that provider-calling processes must be distributed in Wave 0.

The daemon is the machine/provider authority in the current installed topology. If provider calls later move to multiple OS processes, the same typed coordinator contract must move to the machine daemon rather than adding independent process-local limits.

## Contract

`ProviderConfig.backpressure` defines:

- `max_concurrent_requests` (default 2; zero is normalized to one),
- `queue_timeout_ms` (default 120 seconds),
- `max_cooldown_ms` (default 60 seconds).

The stable key is the configured provider name. The first constructed policy for a key is authoritative for that process; validated configuration creates one definition per name. Queue timeout and closed-coordinator errors fail closed with typed stable codes. Retry advice is capped and monotonically extends, never shortens, an active cooldown.

Metrics remain separate from inference rounds, retries, tool calls, and context occupancy: admitted, queued, rejected, cooldown updates, active, and available permits.

## Data flow

```text
main / role child / routed caller / embedding adapter
                         |
                  canonical factory
                         |
             provider-keyed coordinator
          cooldown -> FIFO permit -> transport
                         |
           terminal response / stream drop
                         |
                    release permit
```

## Acceptance

1. Two independently resolved instances with one provider key share one permit.
2. A provider-advised cooldown blocks the next caller and is capped/observable.
3. Queue deadline fails closed and increments rejection evidence.
4. Canonical config schema is deterministic.
5. Installed acceptance observes child fan-out without provider request overlap beyond the configured limit and reports provider retries separately.
