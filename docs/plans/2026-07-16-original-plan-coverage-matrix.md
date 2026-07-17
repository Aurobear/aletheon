# Active Architecture Plan Status

> **Status:** Active traceability authority
>
> **Audited:** 2026-07-17 against the current branch

Completed and superseded plans were removed from `docs/plans`; their outcomes
and current code evidence are retained in
`docs/arch/2026-07-plan-completion-ledger.md`. This file lists only work that
still has an unmet acceptance condition.

## Status vocabulary

| Status | Meaning |
|---|---|
| `partial` | Substantial code exists, but at least one source acceptance item is unmet. |
| `open` | The required production acceptance evidence has not been produced. |
| `blocked-by` | This plan's own implementation is present, but its final aggregate gate depends on another active plan. |

## Active plans

| Plan | Status | Remaining acceptance gap | Current evidence |
|---|---|---|---|
| Architecture and coupling optimization | `blocked-by V02` | The code, authority, memory, Agent and deterministic gates are complete; installed production evidence remains outstanding. | Completion requires the production release gate at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md` §15. |
| Dasein–Agora conscious core | `blocked-by V02` | Deterministic conscious-core, memory and indicator evidence passes; the installed production release evidence remains outstanding. | Final cross-domain requirements are at `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md` §15. |
| Executable plan decomposition | `blocked-by V02` | Every implementation slice is closed except the terminal installed-production receipt. | Terminal completion is defined at `docs/plans/2026-07-15-executable-plan-decomposition-design.md` §8. |
| V02 production migration scenarios | `partial` | The distinct-version install/upgrade/restart/rollback lane passes; real Gmail/SubAgent/TUI, injected-failure and aggregate operator receipts remain open. | Installed-host receipt: generated `target/v02-final-candidate-evidence/operator-receipt.json`; remaining receipt requirement: `docs/testing/production-scenarios.md:8-12`. |

## Current execution order

```text
V02 disposable-host acceptance
              |
              +-> architecture/conscious/decomposition aggregate closure
```

## Verification truth

- `cargo test --workspace --all-targets --no-fail-fast` passes on 2026-07-17.
- `cargo clippy --workspace --all-targets -- -D warnings` passes on 2026-07-17.
- Both architecture gates pass; the production scanner reports no additions.
- The V01 acceptance recipe passes and emits `target/acceptance/acceptance.json`.
- V02 static migration/systemd lanes and the distinct-version disposable-host
  install/rollback lane pass; live workflows, injected failures and the aggregate
  operator receipt remain open, and implementation is not substituted for them.

## Completion rule

This program is complete only when every row above is closed, the production
architecture gate passes, all deterministic suites pass, and V02 produces the
required installed-host operator receipt.
