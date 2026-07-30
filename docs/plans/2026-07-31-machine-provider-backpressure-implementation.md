# F1 Machine/Provider Backpressure Implementation Plan

**Design:** `docs/plans/2026-07-31-machine-provider-backpressure-design.md`

1. Add `ProviderBackpressureConfig` to `crates/cognit/src/config/mod.rs` and regenerate `config/schema/aletheon-config.schema.json`.
2. Add provider-keyed shared state, fair permits, shared cooldown, queue deadline, and snapshots in `crates/cognit/src/adapters/inference/backpressure.rs`.
3. Wrap every transport created by `crates/cognit/src/composition/inference_factory.rs`, holding permits through streaming termination.
4. Expose read-only snapshots through the stable Cognit inference facade.
5. Validate focused unit tests, Executive compilation/schema, then installed child-fan-out acceptance.

No consumer may bypass the canonical provider factory. Remote semantic embeddings introduced by A must consume this coordinator rather than owning a second limiter.
