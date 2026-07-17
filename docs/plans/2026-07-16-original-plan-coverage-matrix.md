# Active Architecture Plan Status

> **Status:** Active traceability authority
>
> **Audited:** 2026-07-18 against the current branch and worktree

Completed and superseded plans were removed from `docs/plans`; their outcomes
and current code evidence are retained in
`docs/arch/2026-07-plan-completion-ledger.md`. This file lists only work that
still has an unmet acceptance condition.

## Status vocabulary

| Status | Meaning |
|---|---|
| `partial` | Substantial code exists, but at least one source acceptance item is unmet. |
| `open` | The required production acceptance evidence has not been produced. |
| `external` | Repository implementation exists, but completion requires real-host credentials, drivers or operator evidence. |
| `blocked-by` | This plan's final aggregate gate depends on one or more active plans. |

## Active plans

| Plan | Status | Remaining acceptance gap | Current evidence |
|---|---|---|---|
| Architecture and coupling optimization | `blocked-by R2/R3 + V02` | R2/R3 production arbitration and current-candidate installed production evidence remain outstanding; closure is not gated by V02 alone. | Aggregate acceptance remains defined at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md` §15. |
| Dasein–Agora conscious core | `blocked-by R2/R3 + V02` | The production field-feedback/arbitration slice is active, and current-candidate installed production evidence remains outstanding. | Active slice: `docs/plans/2026-07-17-conscious-r2-r3-production-arbitration.md:47-746`; final cross-domain requirements: `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md` §15. |
| Conscious-core R2/R3 production arbitration | `partial` | Fabric contracts and initial Dasein/metrics/Cognit work exist, but production binding, enforcement, traces, acceptance tests and current deterministic verification are not all closed. | Approved behavior: `docs/plans/2026-07-17-conscious-r2-r3-production-arbitration-design.md:10-35`; implementation tasks: `docs/plans/2026-07-17-conscious-r2-r3-production-arbitration.md:47-746`. |
| Executable plan decomposition | `blocked-by R2/R3 + V02` | The terminal closure depends on the active R2/R3 slice and current-candidate installed-production receipt. | Terminal completion is defined at `docs/plans/2026-07-15-executable-plan-decomposition-design.md` §8. |
| V02 production migration scenarios | `external` | Tasks 1–6 have implementation artifacts; live Gmail/SubAgent/TUI, real failure-driver and aggregate operator receipts for the current candidate remain open. | Historical only: `target/v02-final-candidate-evidence/operator-receipt.json:1-19`; current receipt requirements: `docs/testing/production-scenarios.md:8-12,109-159`. |

## Current execution order

```text
R2/R3 production arbitration ----> current deterministic verification
              |                                  |
              +----------------------------------+
                                                 |
V02 current-candidate live acceptance -----------+
                                                 |
                                                 +-> aggregate closure
```

## Verification truth

- Historical 2026-07-17 workspace tests, strict Clippy and architecture-gate
  results predate the active R2/R3 branch work and are not current verification.
- The generated V01 report identifies commit `5af244a3a59af82f68d389eb57025a8500264df9`
  at `target/acceptance/acceptance.json:1-6`; it is historical rather than a
  receipt for the current branch/worktree.
- The installed-host receipt records candidate SHA-256
  `9628e35e31c8419672fd93305934b38e4a5bb01fb975279b923e1617f9e4e4be`
  at `target/v02-final-candidate-evidence/operator-receipt.json:6-11`; it does
  not validate the current release binary.
- V02 migration, systemd, live-scenario, failure and aggregate gate
  implementations are present, but live workflows, injected failures and the
  aggregate operator receipt remain external/open. Implementation is not
  substituted for current production evidence.
- No new deterministic or production validation was run during this 2026-07-18
  status reconciliation.

## Completion rule

This program is complete only when every row above is closed, the production
architecture gate and deterministic suites pass for the current revision, and
V02 produces the aggregate current-candidate operator receipt required by
`scripts/release-acceptance.sh:444-462`.
