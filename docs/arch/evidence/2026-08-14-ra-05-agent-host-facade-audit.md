# RA-05 Agent host facade audit

Requirement: the rich Agent host service remains open until production uses
one-way facades (`docs/plans/2026-08-12-migration-closeout-execution-plan.md:205-211`).
The required audit categories and narrow-slice rule are defined at
`docs/plans/2026-08-12-migration-closeout-execution-plan.md:275-299`.

## Classification

| Category | Fields / methods | Evidence |
|---|---|---|
| `RUNTIME_AUTHORITY` | `runtime_agent_supervisor`; generation-fenced spawn/wait/send/cancel and lifecycle receipts | `crates/executive/src/application/agent_control/mod.rs:127-130`; `crates/executive/src/application/agent_control/runtime_bridge.rs:65-95` |
| `HOST_ADAPTER` | `kernel`, clock, SQL projection, admission lease, event spine/projections, live task/mailbox/process effects, recovery and shutdown | `crates/executive/src/application/agent_control/mod.rs:97-118`; `crates/executive/src/application/agent_control/execution_runner.rs:6-407` |
| `APPLICATION_POLICY` | cognitive admission and durable-memory projection remain host inputs; profiles, runtime requirements and historical preference now live in Runtime's effect-free selection policy | `crates/executive/src/application/agent_control/mod.rs`; `crates/runtime/src/agent_supervisor.rs` |
| `COMPAT_TEST_ONLY` | `runtimes: CompatibilityRuntimeCatalog`, legacy constructor and direct convenience methods | `crates/executive/src/application/agent_control/mod.rs:101-104,1264-1368`; production uses `new_runtime_only` at `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:502-548` |
| `UNKNOWN` | none for this slice | Production callers were enumerated with `rg -n 'AgentHostAdapter|RuntimeAgentControlFacade|AgentHostEffects' crates --glob '*.rs'`. |

## Callers

- Production constructs one concrete adapter at
  `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:502-548`.
- Runtime backend registration consumes only `AgentHostEffects` at
  `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:550-594`.
- Public Agent commands consume `AgentControlPort` through
  `RuntimeAgentControlFacade` at
  `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:631-635`.
- Before this slice, recovery/live-run/shutdown still called the concrete
  adapter at `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:596-660`.
- Remaining direct concrete callers are Executive compatibility tests found by
  `rg -n 'AgentHostAdapter::new_(legacy|runtime_only)' crates/executive/tests`.

## Implemented narrow slice

`RuntimeAgentLifecycleFacade` now owns the production recovery, live-run and
shutdown view. `AgentHostFacades` projects command, backend-effect and lifecycle
views in one operation, so bootstrap no longer calls rich service methods after
composition. Profile resolution, capability requirements, and historical
Runtime preference moved into Runtime's `AgentRuntimeSelectionPolicy` behind
the `RuntimePreferenceHistory` query port. Production then consumes the
adapter with `into_facades()`, so bootstrap cannot retain the rich concrete
object beside those three views. Cognitive workspace admission and process
execution remain host effects rather than being copied into Runtime.

This composition type boundary closes `RA-A-10`; the result is backed by the
focused spawn/recovery tests rather than a grep-only assertion. Removing the
remaining compatibility-fixture API is deferred to RA-06/XRET.
