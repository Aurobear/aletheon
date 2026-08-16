# D6 Runtime EventSpine cutover evidence

- Date: 2026-08-12
- Canonical owner: `runtime::event_spine`.
- The durable event identity, causal position, visibility/payload contracts, unsequenced/sequenced events, and append/read port moved together to Runtime Journal.
- Runtime, Aletheon composition, Aletheon Extension, Corpus and Executive adapters/application callers use Runtime directly.
- Fabric's implementation, module/root re-exports, contract test and nine census/public rows were deleted without a compatibility facade.
