# XRET-05 platform deployment owner cutover — 2026-08-13

## Requirement anchor

The migration closeout requires the Executive package to be empty before its
physical deletion.
The governed surface ledger assigned the former Executive deployment module to
an operating-system adapter owner (`config/architecture/executive-surface-ledger.tsv`,
former `crates/platform/src/deployment/mod.rs` row).

## Cutover

- Moved deployment manifest inspection, provenance comparison, rollback planning,
  and the bounded filesystem rollback implementation to
  `crates/platform/src/deployment/mod.rs`.
- `aletheon` now consumes deployment diagnostics and rollback composition from
  `platform`; Executive retains only a temporary re-export for its remaining
  internal/test callers.
- Removed the retired Executive source from the Executive layer and surface
  inventories.
- Removed the obsolete `basic_agent -> executive` dependency. The example now
  demonstrates typed configuration initialization without constructing the
  legacy Executive facade.

This is a one-way owner cutover toward XRET-05, not a claim that XRET-05 is
complete. Production `aletheon` still has substantial Executive application and
orchestrator callers.

## Validation

- `bash scripts/cargo-agent.sh test -p platform --lib`: 46 passed.
- `bash scripts/cargo-agent.sh check -p executive --lib`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
- `bash scripts/cargo-agent.sh test -p basic_agent --bins`: passed.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture`:
  652 governed contract types, no additions.
- `bash scripts/cargo-agent.sh fmt --all -- --check`: passed.
- `git diff --check`: passed.
