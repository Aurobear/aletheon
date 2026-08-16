# APX thread-authority owner cutover — 2026-08-13

The Executive surface ledger assigns the mixed thread-authority surface to
Application plus host adapters (`config/architecture/executive-surface-ledger.tsv`,
`application/thread_authority.rs`). The authoritative typed settings, immutable
principal/thread binding, persistent store, and errors now live in
`crates/application/src/thread_authority.rs`.

All Aletheon production callers import `application::thread_authority` directly.
The former Executive module is a temporary compatibility re-export for remaining
Executive-internal callers and contains no implementation.

Validation:

- `bash scripts/cargo-agent.sh test -p application --lib`: 41 passed.
- `bash scripts/cargo-agent.sh check -p executive --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
