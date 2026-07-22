# Core Architecture Refactor Completion Report

**Status:** implemented and verified

**Architecture source:** `CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:940-954` (global verification), `:958-975` (compatibility/security), and `:1018-1040` (18 acceptance criteria).

## Result

The refactor is complete. Core orchestration now depends on capability ports, concrete integration vocabulary is confined to adapter/deployment/compatibility boundaries, public implementation trees are closed, large state machines have explicit owners, and all mandatory serial validation lanes pass.

```text
host/composition -> adapters -> external systems
       |              ^
       v              |
 application ------> ports
       |
       v
 domain/contracts      (no infrastructure ownership)
```

## Metrics delta

| Metric | Phase 0 | Final | Assessment |
|---|---:|---:|---|
| Core external-identifier hits | 252 | 33 | -86.9%; remaining counted vocabulary is adapter/compatibility-boundary material |
| Public impl/adapter exports | 24 | 0 | target reached |
| Cross-crate impl references | 12 | 0 | target reached |
| Forbidden infrastructure imports | 20 | 8 | -60%; remaining counted imports are reviewed boundary/false-positive patterns, with no new findings |
| Fabric provider-specific types | not separately frozen | 0 | target reached |
| Provider-name branches | 0 | 0 | target held |
| URL provider inference | 0 | 0 | target held |
| Provider error-text branches | 0 | 0 | target held |
| Opaque-value inspections | 2 | 2 | bounded reviewed compatibility parsing, not business branching |
| Compatibility ledger rows | 19 | 2 | only persisted ExternalEvent v1 read compatibility remains |

Final ratchets are authoritative at `config/architecture/metrics.env:2-10`; the remaining compatibility exits are explicit at `config/architecture/compatibility-debt.tsv:3-4`.

## Acceptance audit

| # | Criterion | Authoritative evidence | Result / remaining risk |
|---:|---|---|---|
| 1 | Fabric has no provider-specific shared types/scopes/errors | `FABRIC_PROVIDER_TYPES=0` (`config/architecture/metrics.env:5`); provider identity is opaque `ExternalProviderId` (`crates/fabric/src/types/external_identity.rs:51-76`) | pass; v1 serialized aliases remain bounded compatibility |
| 2 | Domain/application access I/O only through ports | Executive layer inventory (`config/architecture/executive-layers.tsv:62-80`); inference port (`crates/executive/src/application/inference_port.rs:29`); architecture fixtures | pass |
| 3 | Goal and Agent Control are runtime-name neutral | Goal coordinator (`crates/executive/src/application/goal/coordinator.rs:79`), Agent Control owner (`crates/executive/src/application/agent_control/mod.rs:126`), runtime adapters under `crates/executive/src/adapters/runtime/` | pass |
| 4 | Cognit is channel neutral | architecture external-name and dependency gates; channel implementations reside under Executive/Gateway adapters | pass |
| 5 | Memory core is supplemental-product neutral | generic service port (`crates/mnemosyne/src/composite_service.rs:28`) and transport port (`crates/mnemosyne/src/backends/supplemental/backend.rs:71`) | pass |
| 6 | Hardware core is ROS/vendor/simulator neutral | generic execution/provider traits (`crates/fabric/src/types/embodiment.rs:110`, `crates/hardware/src/skill.rs:59`); provider-specific type metric is zero | pass; concrete simulator/grpc exports remain boundary utilities, not domain branching |
| 7 | Adding a provider does not modify a core enum | opaque provider IDs (`crates/fabric/src/types/external_identity.rs:51-76`); provider-name branch metric zero | pass |
| 8 | Provider failures reach core as generic classes | generic `FailureClass` (`crates/fabric/src/types/attempt.rs:41-61`); provider error-text branch metric zero | pass |
| 9 | Config, secret resolution, adapter construction are composition-owned | config ownership inventory (`config/architecture/config-ownership.tsv:3-24`); Executive composition tree | pass |
| 10 | Domain/application do not own HTTP/DB/home-dir infrastructure | dependency architecture gate and layer inventory; concrete SQLite repositories are classified adapters (`config/architecture/executive-layers.tsv:3-61`) | pass |
| 11 | Crate roots do not expose implementation/adapter trees | public impl/adapter export metric zero (`config/architecture/metrics.env:9`); cross-crate impl references zero (`:4`) | pass |
| 12 | Legacy config/persistence/API has compatible or explicit rejection behavior | compatibility/security audit (`evidence/phase-10/11-compatibility-security-audit.md`) and wire/persistence inventories | pass; two v1 read aliases have explicit data-driven exits |
| 13 | Architecture checks prevent recurrence | architecture check and negative fixtures (`evidence/phase-10/01-architecture-check.log`, `02-architecture-fixtures.log`) | pass |
| 14 | Core use cases are testable with fake ports | green workspace tests include Goal, Agent Control, memory, provider and channel fake-port suites (`evidence/phase-10/08-workspace-test.log`) | pass |
| 15 | Replacing an implementation is adapter/config/contract-test scoped | generic ports above, zero provider-name branching, adapter classification inventory | pass |
| 16 | Large state machines have unique owners/transitions | five-owner inventory (`config/architecture/state-machine-inventory.tsv:2-6`) | pass |
| 17 | Every migration phase has narrow evidence | phase plans and committed phase-specific tests; Phase 10 artifacts under `docs/arch/evidence/phase-10/` | pass |
| 18 | Workspace verification follows Rust resource policy | every recorded Cargo command used `scripts/cargo-agent.sh`; serial lane evidence below | pass |

## Serial validation lanes

| Lane | Result | Duration | Evidence |
|---|---|---:|---|
| Architecture check | pass | 1.89s | `evidence/phase-10/01-architecture-check.log` |
| Architecture negative fixtures | pass | 1.17s | `evidence/phase-10/02-architecture-fixtures.log` |
| Architecture path inventory | pass | 1.95s | `evidence/phase-10/03-path-inventory.log` |
| Operations CLI static boundary | pass | 0.07s | `evidence/phase-10/04-operations-cli.log` |
| systemd runtime boundary | pass | 0.27s | `evidence/phase-10/05-systemd-boundary.log` |
| rustfmt | pass | 1.62s | `evidence/phase-10/06-fmt.log` |
| Workspace check/all targets | pass | 45.32s | `evidence/phase-10/07-workspace-check.log` |
| Workspace test | pass | 330.44s | `evidence/phase-10/08-workspace-test.log` |
| Clippy/all targets/deny warnings | pass | 34.47s | `evidence/phase-10/09-clippy.log` |
| Workspace rustdoc | pass | 29.25s | `evidence/phase-10/10-workspace-doc.log` |

## Stateful-module review

State ownership is explicit, but the largest I/O/orchestration files remain sizeable: Agent Control settlement, turn pipeline, daemon bootstrap/server, Goal attempt coordination, and MCP client. The inventory at `config/architecture/state-machine-inventory.tsv:2-6` provides unique mutation entry points. Further splitting should be local module extraction only when a concrete change hotspot appears; reopening cross-crate ownership is not justified by size alone.

## Crate-split decision

| Crate | Fan-in / fan-out | Public API count (mechanical `pub` items) | Ownership/release evidence | Decision |
|---|---|---:|---|---|
| Fabric | 14 / 0 workspace crate edges | 1200 | Stable shared contracts have broad fan-in; no provider-specific types or dependency cycle, and no independent release need | **keep** |
| Executive | 4 / 11 workspace crate edges | 1195 | Composition root legitimately has broad fan-out; application/adapter/host boundaries and zero cross-crate impl imports give isolation without a new version boundary | **keep** |

A physical split would multiply version coordination and migration cost without evidence of an ownership, release, or cycle benefit. This follows the decision rule at `CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:940-954`. If future incremental timing or ownership data changes, start a separate design; do not append a split to this completed phase.

## Residual risks

1. Two persisted-v1 aliases cannot safely be deleted until the supported upgrade window is formally closed and deployed data shows no v1 rows (`config/architecture/compatibility-debt.tsv:3-4`).
2. Mechanical public-item counts are intentionally conservative and include protocol/domain surfaces; future API reduction should be compatibility-led rather than visibility churn.
3. Large orchestration files remain maintainability hotspots, but their mutation ownership is now explicit and guarded.
