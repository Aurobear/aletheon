# XRET-05 Aletheon runtime-facade production cutover — 2026-08-13

## Requirement anchor

XRET-05 requires eventual physical deletion of the empty Executive crate
(`docs/plans/2026-08-12-migration-closeout-execution-plan.md` §12 item 5).
The surface ledger assigns `core/orchestrator.rs` to Runtime and splits its
mixed evolution/mode concerns among their owners
(`config/architecture/executive-surface-ledger.tsv`, corresponding core rows).
The binary composition root is already owned by `crates/aletheon/src/wiring`.

## Cutover

The production daemon no longer obtains `AletheonExecutive`, `ModeRouter`, or
`EvolutionCoordinator` through the Executive facade. Their current composition
implementations are now binary-owned under:

- `crates/aletheon/src/wiring/executive_runtime.rs`
- `crates/aletheon/src/wiring/mode_router.rs`
- `crates/aletheon/src/wiring/evolution_coordinator.rs`

`request.rs`, `request_ports.rs`, and `services.rs` consume those binary-owned
modules. Executive copies remain temporarily for Executive-only compatibility
tests; they are not production callers and must be deleted with those tests in
the final XRET-05 physical deletion.

This cutover removes a production dependency seam but does not claim Executive
caller-zero or XRET-05 completion.

## Validation

- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
