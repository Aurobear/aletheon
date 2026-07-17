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
| Architecture and coupling optimization | `partial` | Q02, G08, M05, M06 and V02 still prevent the definition of architectural completion. | Completion requires all authoritative facades and gates at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md` §15. |
| Dasein–Agora conscious core | `partial` | Memory consolidation/reconciliation and aggregate indicator/release evidence remain incomplete. | Final cross-domain requirements are at `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md` §15. |
| Executable plan decomposition | `partial` | The decomposition cannot close until Q02 and V02 close and this matrix has no active rows. | Terminal completion is tracked by this table. |
| E01 architecture fitness baseline | `partial` | The production architecture gate is red because Bin directly depends on Fabric. | Rejection is encoded at `scripts/architecture-check.sh:37-50`; the edge is at `crates/bin/Cargo.toml:16-20`. |
| G08 Agent admission and budgets | `partial` | Capacity exhaustion returns immediately; the required fair waiting queue and eventual admission are absent. | Immediate rejection: `crates/executive/src/service/agent_control/admission.rs:187-195`; current acceptance behavior: `crates/executive/tests/agent_admission.rs:79-118`. |
| M05 leased memory consolidation | `partial` | The production path does not enqueue extraction, so experience-to-candidate consolidation is not closed. | Consolidation worker only supervises consolidation at `crates/executive/src/service/memory_consolidation_worker.rs:5-38`; repository lease machinery exists at `crates/mnemosyne/src/consolidation/repository.rs:124-175`. |
| M06 GBrain reconciliation | `partial` | Executive still owns claim, receipt construction and retry/dead-letter settlement that belong behind Mnemosyne. | Ownership leak: `crates/executive/src/impl/gbrain/worker.rs:62-142`; intended domain operations: `crates/mnemosyne/src/backends/gbrain/reconcile.rs:9-115`. |
| Mnemosyne unified memory | `partial` | M05, M06 and the plan's dedicated observability metrics remain incomplete. | Release criteria remain at `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md` §10. |
| Q02 typed Interact/thin Bin | `partial` | Remove Bin's Fabric dependency/direct protocol knowledge and restore the architecture gate. | Forbidden edge: `crates/bin/Cargo.toml:16-20`; gate: `scripts/architecture-check.sh:37-50`. |
| S02 unified turn coordinator | `blocked-by Q02` | Core lifecycle/equivalence tests pass, but its required repository architecture gate is red. | Coordinator acceptance: `crates/executive/tests/turn_coordinator_lifecycle.rs:41-166`; final gate requirement: `docs/plans/2026-07-15-s02-unified-turn-coordinator.md` §Compatibility deletion gate and completion evidence. |
| SubAgent unified harness | `partial` | G08 remains partial; Pi still lacks pinned protocol/build identity, strict JSONL typed parsing and RPC lifecycle mapping. | Opaque Pi stdout handling: `crates/executive/src/impl/runtime/pi.rs:444-458`. |
| V01 cross-domain acceptance | `blocked-by Q02` | Functional/ablation tests exist, but the aggregate acceptance command cannot pass while E01 is red. | Indicators: `crates/executive/tests/functional_indicators.rs:73-120`; deterministic acceptance: `crates/executive/tests/cross_domain_acceptance.rs:20-61`. |
| V02 production migration scenarios | `open` | Run the disposable-host install/upgrade/restart/failure/rollback scenarios and retain a passing operator receipt. | Receipt requirement: `docs/testing/production-scenarios.md:8-12`; release harness: `scripts/release-acceptance.sh:111-155`. |

## Current execution order

```text
Q02 -> E01 -> S02 -> V01
G08 --------------------+
M05 -> M06 -------------+-> architecture/conscious/memory aggregate closure
                         +-> V02 disposable-host acceptance
```

## Verification truth

- Domain crate all-target tests audited for Mnemosyne, Agora, Dasein, Kernel and Executive pass.
- Focused Q01, R01, R02, S01, X01, X02 and functional-indicator suites pass.
- `bash tests/architecture_check.sh` passes its fixture/runtime boundary checks.
- `bash scripts/architecture-check.sh` currently fails on the Bin-to-Fabric edge at `crates/bin/Cargo.toml:16-20`; this failure is intentionally retained as active E01/Q02 evidence.

## Completion rule

This program is complete only when every row above is closed, the production
architecture gate passes, all deterministic suites pass, and V02 produces the
required installed-host operator receipt.
