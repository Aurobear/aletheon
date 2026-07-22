# H5 SQLite migration resilience evidence — 2026-07-22

## Requirement anchors

- GBrain migration must first verify SQLite DDL transaction behavior, inject interruption/re-entry,
  and then establish an explicit transaction boundary
  (`docs/plans/2026-07-21-production-readiness-hardening.md:196-201`).
- SessionStore needs an evolvable database-structure version while retaining record-schema checks
  (`docs/plans/2026-07-21-production-readiness-hardening.md:202`).
- Integrity diagnosis must remain offline or controlled and must not add an unconditional full scan
  to every open (`docs/plans/2026-07-21-production-readiness-hardening.md:203-204`).
- Every migration-step failure must either reopen successfully or fail closed, versions may advance
  only after schema/data success, and legacy session/record fixtures are required
  (`docs/plans/2026-07-21-production-readiness-hardening.md:206-211`).

## Implemented database contracts

```text
open database
    |
    +-- version > supported ------------------------> fail closed, no mutation
    |
    +-- old version --> BEGIN IMMEDIATE
    |                    +-- schema / columns
    |                    +-- data backfill
    |                    +-- user_version last
    |                    `-- COMMIT (any error rolls all steps back)
    |
    `-- supported version --> required-column probe --> ready / fail closed

offline operator --> aletheon-sqlite-check.sh --> read-only PRAGMA quick_check
```

- GBrain performs all table, column, backfill, and version changes in one `IMMEDIATE` transaction;
  it refuses a newer schema before entering that transaction
  (`crates/mnemosyne/src/backends/gbrain/migrations.rs:25-182`).
- The GBrain fixture injects failure after all 14 observable steps. Every case retains v1, then
  reopens and completes v2 with its backfilled logical identity intact; a v99 fixture remains
  untouched (`crates/mnemosyne/src/backends/gbrain/migrations.rs:205-294`).
- Canonical SessionStore introduces database schema v1 independently from
  `SESSION_SCHEMA_VERSION`, commits its marker last, and validates the required v1 columns even
  when the database already claims v1
  (`crates/executive/src/impl/session/canonical_store.rs:19-139`).
- Session fixtures cover both transactional failure boundaries, an unversioned legacy database
  with an unchanged record JSON, independent session/item record rejection, a newer database, and
  an incomplete claimed-v1 database (`crates/executive/src/impl/session/canonical_store.rs:397-575`).

## Controlled integrity-check decision

No production-size latency evidence or corruption incident justifies a scan on every daemon/store
open. H5 therefore does **not** call `quick_check` or `integrity_check` from either open path.
Operators instead have an explicit read-only command that rejects missing paths and fails unless
SQLite returns exactly `ok` (`scripts/aletheon-sqlite-check.sh:1-37`):

```bash
# Stop the owning service first, or point at consistent backup copies.
bash scripts/aletheon-sqlite-check.sh /path/to/sessions-v1.db /path/to/gbrain-spool.db
```

This keeps the size-dependent diagnostic outside the high-frequency path while making it
repeatable during maintenance and migration rehearsals.

## Deterministic verification

Commands are serialized through the repository Cargo wrapper:

```bash
bash scripts/cargo-agent.sh test -p mnemosyne backends::gbrain::migrations --lib
bash scripts/cargo-agent.sh test -p mnemosyne --test gbrain_reconciliation opens_and_upgrades_previous_schema_fixture_forward_only
bash scripts/cargo-agent.sh test -p executive r#impl::session::canonical_store --lib
bash scripts/cargo-agent.sh test -p executive --test session_append_store
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/architecture-check.sh
git diff --check
```

Observed results:

- GBrain migration unit tests: 2 passed, including all 14 failure points;
- GBrain previous-schema integration fixture: 1 passed;
- Canonical SessionStore unit tests: 8 passed, including migration, legacy record, fail-closed, and
  existing recovery behavior;
- session append integration target: 3 passed;
- offline script: healthy fixture passed; missing database was rejected before SQLite invocation.

## Compatibility and rollback

- GBrain remains schema v2; H5 changes atomicity but does not introduce a new on-disk version.
- SessionStore tables and record JSON are unchanged. H5 only records `user_version=1`; an H4-era
  binary ignores that pragma and continues to use the same tables, so rollback is backward
  compatible.
- Roll back the independent H5 commit if required. Do not delete database files; first stop the
  owner, retain a consistent copy, and use the explicit quick-check diagnostic.
