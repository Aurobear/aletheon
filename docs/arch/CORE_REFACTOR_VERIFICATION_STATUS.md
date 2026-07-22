# Core Refactor Phase 10 Verification Status

**Status:** incomplete — architecture gates, formatting, workspace check, and rustdoc pass; the mandatory workspace test and `-D warnings` Clippy lanes are not green.

## Requirement anchors

The final phase must validate measurable architecture gains rather than assume a crate split (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:940-954`). The overall acceptance criteria are listed at `CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:1018-1040`. Serial commands and the rule that every zero target must actually reach zero are specified by `docs/plans/core-refactor/11_PHASE_10_GLOBAL_VERIFICATION.md:55-91`.

## Metric delta

| Metric | Phase 0 | Current | Result |
|---|---:|---:|---|
| core external-identifier hits | 252 | 33 | reduced 86.9%; remaining hits are counted adapter/compatibility vocabulary |
| public impl/adapter exports | 24 | 0 | pass |
| cross-crate impl references | 12 | 0 | pass; examples are now included in the gate |
| forbidden infrastructure imports | 20 | 8 | reduced 60%; remaining entries require criterion-by-criterion audit |
| Fabric provider-specific types | not separately frozen | 0 | pass |
| provider-name branches | 0 | 0 | pass |
| URL provider inference | 0 | 0 | pass |
| provider error-text branches | 0 | 0 | pass |
| opaque-value inspections | 2 | 2 | unchanged counted debt |
| compatibility ledger entries | 19 initial data rows | 4 | Phase 9 exits are zero; four durable Phase 10 migrations remain |

Current metric source: `config/architecture/metrics.env:1-10`. Original values are preserved by commit `c9a46e9`.

## Serial validation lanes

| Lane | Result | Duration | Evidence |
|---|---|---:|---|
| architecture check | pass | 1.89s | `evidence/phase-10/01-architecture-check.log` |
| architecture fixtures | pass | 1.17s | `evidence/phase-10/02-architecture-fixtures.log` |
| path inventory | pass | 1.92s | `evidence/phase-10/03-path-inventory.log` |
| operations CLI static | pass | 0.07s | `evidence/phase-10/04-operations-cli.log` |
| systemd boundary | pass | 0.27s | `evidence/phase-10/05-systemd-boundary.log` |
| rustfmt check | pass after formatting commit | 1.61s | `evidence/phase-10/06-fmt.log` |
| workspace check/all targets | pass after downstream facade fixes | 7.78s warm | `evidence/phase-10/07-workspace-check.log` |
| workspace tests | **fail** | 456.85s | `evidence/phase-10/08-workspace-test.log` |
| Clippy, all targets, deny warnings | **fail** | 22.26s | `evidence/phase-10/09-clippy.log` |
| workspace rustdoc | pass | 50.74s | `evidence/phase-10/10-workspace-doc.log` |

### Workspace-test failure classification

The full lane reached `corpus::hook::registry::tests::execute_script_hook_block`, which returned `Continue` rather than `Block` under workspace load. The exact isolated test passed immediately afterward. This is evidence of a load-sensitive/flaky hook execution test, not evidence of a green workspace lane; therefore the mandatory lane remains failed.

An earlier run also exposed a real legacy-config merge defect and stale facade imports. Those were fixed in commit `8fa260a`; the failing Aletheon end-to-end test and the new legacy normalization regression test both pass.

### Clippy failure classification

Clippy stops in Fabric with 46 pre-existing `clippy::uninlined_format_args` errors before reaching the rest of the workspace. Because the Phase 10 command explicitly uses `-D warnings`, these cannot be reported as pass or silently waived. The full log is retained for a bounded lint-debt cleanup stage.

## Crate-split decision (provisional)

| Crate | Decision | Evidence and risk |
|---|---|---|
| Fabric | keep | provider-specific shared types and provider branching are zero; a physical split would add release/versioning cost without demonstrated cycle or ownership benefit |
| Executive | keep | explicit application/composition/host boundaries and zero cross-crate impl imports provide isolation; current build cost is high, but no independent release boundary has been demonstrated |

These decisions remain provisional until the failed validation lanes and the remaining acceptance/security audit are closed. The architecture plan status must not be changed to implemented yet.
