# D2 Cognit inference-port owner cutover — 2026-08-13

## Requirement anchor

The Executive surface ledger assigns
`crates/cognit/src/ports/inference.rs` to `cognit/ports` via D2
(`config/architecture/executive-surface-ledger.tsv`, inference-port row).
XRET-05 ultimately requires all production Executive callers to be zero.

## Cutover

The provider-neutral core inference request, capabilities, error, port,
backpressure bridge, local adapter, and legacy-provider presentation adapter now
live in `crates/cognit/src/ports/inference.rs`. All Aletheon production and
integration callers import the Cognit owner directly. The former Executive
module is now only a compatibility re-export for remaining Executive-internal
application modules; it contains no implementation.

Exact search after cutover finds zero
`executive::application::inference_port` references in `crates/aletheon/src`.

## Validation

- `bash scripts/cargo-agent.sh check -p cognit --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p executive --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
