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
| V02 production migration scenarios | `open` | Run the disposable-host install/upgrade/restart/failure/rollback scenarios and retain a passing operator receipt. | Receipt requirement: `docs/testing/production-scenarios.md:8-12`; release harness: `scripts/release-acceptance.sh:111-155`. |

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
- V02 static migration, systemd and monitor lanes pass, but no disposable-host
  operator receipt exists; implementation is not substituted for that evidence.

## Completion rule

This program is complete only when every row above is closed, the production
architecture gate passes, all deterministic suites pass, and V02 produces the
required installed-host operator receipt.
