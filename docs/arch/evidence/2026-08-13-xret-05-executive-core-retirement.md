# XRET-05 Executive core tree retirement — 2026-08-13

## Requirement anchor

XRET-05 requires physical deletion of the Executive crate after its owned
surfaces have moved and callers are zero
(`docs/plans/2026-08-12-migration-closeout-execution-plan.md` §12 item 5).
The Executive surface ledger classifies `core/mod.rs` for deletion and assigns
each former child to an owner or deletion gate.

## Result

The entire `crates/executive/src/core` tree is now absent.

- Deployment ownership moved to `platform::deployment`.
- The live runtime facade, mode router, and evolution coordinator moved to the
  binary-owned Aletheon composition tree; their tests moved with them.
- The legacy checkpoint, `DomainPorts`, verdict handler, and SessionGateway
  trees had no production caller and were deleted.
- The obsolete SessionGateway debug trait implementation was removed; typed
  gateway/application ports remain the supported path.
- Source-scanning architecture tests now assert the current owner paths rather
  than requiring retired Executive files to exist.

This proves Executive **core-tree** caller-zero, not full Executive crate
caller-zero. Application and adapter surfaces remain.

## Validation

- `bash scripts/cargo-agent.sh test -p aletheon --test genome_runtime_mapping --test evolution_integration`: 11 passed.
- `bash scripts/cargo-agent.sh check -p executive --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed before
  final core-tree deletion and will be repeated in the next aggregate gate.
