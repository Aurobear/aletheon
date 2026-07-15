# Executable Architecture Plan Decomposition Design

> **Status:** Proposed decomposition approved in conversation
>
> **Source baseline:** `65f74981`
>
> **Purpose:** Convert the four architecture-scale proposals into small, ordered, independently verifiable implementation plans.

## 1. Source requirements

This decomposition preserves, rather than replaces, the following designs:

- Architecture convergence and single authoritative paths: `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:34-48`.
- Capability execution before broader lifecycle migration: `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:958-979`.
- Canonical Session/Turn/Item lifecycle: `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:981-1006`.
- Dasein state engine and persistence: `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:782-804`.
- Agora transaction integrity and broadcast: `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:806-845`.
- Recurrent Dasein–Agora integration: `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:847-884`.
- Mnemosyne canonical records, recall, projection and lifecycle: `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:302-575`.
- SubAgent control, Native Cognit runtime, communication and recovery: `docs/plans/2026-07-15-subagent-unified-harness-plan.md:402-708`.

## 2. Current-code anchors

The first plans must address verified current boundaries:

- `CoreSystems` exposes concrete runtime/domain groups: `crates/executive/src/core/core_systems.rs:33-68`.
- `DefaultCapabilityInvoker` declares the production invariant but is not wired by Executive: `crates/kernel/src/capability/mod.rs:1-5`, `crates/kernel/src/capability/mod.rs:27-50`.
- Dasein construction hard-codes temporal retention: `crates/dasein/src/dasein/mod.rs:57-65`.
- Sorge uses concrete `SystemTimer`: `crates/dasein/src/dasein/sorge.rs:72-80`, `crates/dasein/src/dasein/sorge.rs:151-163`.
- Agora commit authorization is only a boolean plus process ID: `crates/fabric/src/include/agora.rs:124-140`.
- Agora claims do not validate ownership on claim/release: `crates/agora/src/workspace/mod.rs:227-233`.
- Local SQLite is the default memory authority and GBrain is supplemental/disabled: `config/default.toml:25-40`.

## 3. Decomposition rules

Every generated implementation plan must satisfy all of these rules:

1. One plan produces one reviewable vertical change and a green default test suite.
2. A plan may create a compatibility adapter, but must state its deletion gate.
3. Each task is a 2–5 minute TDD action with an exact path, symbol, test command and expected result.
4. Every plan names its prerequisites and downstream unlocks.
5. Shared contracts land before production consumers.
6. No plan invents a second Session, capability, memory, workspace or Agent authority.
7. Production remains deployable after every plan.
8. Real installed-binary scenarios are used only after deterministic unit/integration coverage.
9. Generated plans use compact ASCII diagrams, not Mermaid.
10. Each plan ends with scoped checks, workspace checks and a commit boundary.

## 4. Master execution DAG

```text
E01 architecture fitness baseline
 |
E02 Corpus ToolExecutor adapter
 |
E03 governed CapabilityInvoker production path
 |
S01 Session/Turn/Item contracts and canonical append store
 |
S02 one TurnCoordinator for daemon and exec
 |
 +----------------------------+----------------------------+
 |                            |                            |
M01 memory behavior baseline  G01 subagent baseline       D01 Dasein config/timer truth
 |                            |                            |
M02 canonical records/scopes  G02 AgentControl contracts  D02 Self reducer contracts
 |                            |                            |
M03 unified local recall      G03 AgentControl service    D03 Self ledger/replay
 |                            |
 |                            G04 Native Cognit runtime
 |                            |
 |                            G05 thin Agent tools + mailbox
 |                            |
A01 Agora transaction and permit integrity
 |
A02 typed candidates and deterministic selection
 |
A03 durable broadcast and subscriber delivery
 |
C01 Recurrent workspace coordinator
 |\
 | +--> M04 bounded memory projection/candidates
 | +--> G06 child context and Agora candidates
 |
M05 leased extraction and consolidation
 |
M06 GBrain reconciliation
 |
M07 retention and forgetting
 |
G07 live mailbox -> G08 admission/budgets -> G09 scoped memory -> G10 recovery
 |
C02 Metacog/Corpus/SubAgent processor integration
 |
V01 deterministic cross-domain acceptance suite
 |
V02 installed-daemon real scenario and migration gate
```

`D01-D03`, `K01-K02`, `X01-X02`, `R01-R02`, and `Q01-Q02` can progress beside `M01-M03` and `G01-G04` after `S02`. `A01` must land before candidate/broadcast production wiring. `C01` requires `D03`, `A03`, `K02`, `X02`, and `S02`. The complete source-to-plan trace is maintained in `2026-07-16-original-plan-coverage-matrix.md`.

## 5. Plan inventory

### Foundation

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| E01 | `2026-07-15-e01-architecture-fitness-baseline.md` | CI dependency/bypass inventory with shrink-only allowlist | none |
| E02 | `2026-07-15-e02-corpus-tool-executor-adapter.md` | Corpus adapter implementing Kernel `ToolExecutor` | E01 |
| E03 | `2026-07-15-e03-governed-capability-invoker.md` | One governed invoker used by daemon and exec | E02 |
| S01 | `2026-07-15-s01-session-turn-item-contracts.md` | Versioned lifecycle contracts and canonical append store | E01 |
| S02 | `2026-07-15-s02-unified-turn-coordinator.md` | Daemon and exec enter the same coordinator and invoker | E03, S01 |

### Mnemosyne

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| M01 | `2026-07-16-m01-memory-contract-baseline.md` | Record/reopen/recall/outage contract tests | S02 |
| M02 | `2026-07-16-m02-canonical-memory-records-scopes.md` | Canonical records, scopes, validation and adapters | M01 |
| M03 | `2026-07-16-m03-unified-local-recall.md` | Scoped local recall across existing stores | M02 |
| M04 | `2026-07-15-m04-bounded-memory-workspace-projection.md` | One bounded projection entering Agora as candidates | M03, C01 |
| M05 | `2026-07-15-m05-leased-memory-consolidation.md` | Restart-safe extraction and consolidation workers | M04 |
| M06 | `2026-07-15-m06-gbrain-reconciliation.md` | Supplemental reconciliation and durable remote receipts | M05 |
| M07 | `2026-07-15-m07-retention-forgetting.md` | Scoped tombstones, retention and auditable compaction | M06 |
| M08 | `2026-07-15-m08-subagent-memory-isolation.md` | Child scopes and reviewed promotion boundary | M04, G09 |

### SubAgent runtime

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| G01 | `2026-07-16-g01-subagent-production-baseline.md` | Current vertical-slice and known-gap tests | S02 |
| G02 | `2026-07-16-g02-agent-control-contracts.md` | Shared bounded Agent control types and port | G01 |
| G03 | `2026-07-15-g03-agent-control-service.md` | Transactional lifecycle and durable run repository | G02 |
| G04 | `2026-07-15-g04-native-cognit-runtime.md` | Child Agents use Cognit Harness; inline loop removed | G03, E03 |
| G05 | `2026-07-15-g05-agent-tools.md` | Thin spawn/wait/send/cancel/list clients | G04 |
| G06 | `2026-07-15-g06-subagent-context-agora-projection.md` | Bounded context fork and typed child candidates | G05, C01 |
| G07 | `2026-07-15-g07-agent-mailbox.md` | Live bounded mailbox and terminal-state semantics | G06 |
| G08 | `2026-07-15-g08-agent-admission-budgets.md` | Root admission, depth, concurrency and hierarchical budgets | G07, K02 |
| G09 | `2026-07-15-g09-agent-memory-promotion.md` | Scoped child memory and reviewed result promotion | G08, M04 |
| G10 | `2026-07-15-g10-agent-recovery-cleanup.md` | Durable restart recovery, lease reclamation and cleanup | G09 |

### Dasein and Agora

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| D01 | `2026-07-16-d01-dasein-config-timer-lifecycle.md` | Configured temporality and restartable injected timing | S02 |
| D02 | `2026-07-16-d02-dasein-self-reducer.md` | Versioned self transition contracts and reducer | D01 |
| D03 | `2026-07-15-d03-dasein-self-ledger-replay.md` | Complete snapshots, ledger, replay and causal lineage | D02 |
| A01 | `2026-07-15-a01-agora-transaction-integrity.md` | Bound permits, version recheck and ownership-safe claims | S02 |
| A02 | `2026-07-15-a02-agora-candidate-selection.md` | Typed bounded candidates and deterministic selection | A01 |
| A03 | `2026-07-15-a03-agora-broadcast-delivery.md` | Durable epochs, bounded delivery and acknowledgements | A02 |

### Cross-domain integration and release

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| C01 | `2026-07-15-c01-recurrent-workspace-coordinator.md` | Dasein salience ↔ Agora broadcast recurrence | D03, A03, S02 |
| C02 | `2026-07-15-c02-conscious-processors-integration.md` | Memory, Metacog, Corpus and SubAgent processors | M05, G10, C01, F01 |
| V01 | `2026-07-15-v01-cross-domain-acceptance-suite.md` | Deterministic lifecycle, isolation and replay suite | C02, M07, M08, R02, Q02 |
| V02 | `2026-07-15-v02-production-migration-scenarios.md` | Installed-daemon scenarios, rollback and release gates | V01 |

### Remaining architecture migration

| ID | Plan artifact | Deliverable | Prerequisites |
|---|---|---|---|
| K01 | `2026-07-15-k01-kernel-runtime-contracts.md` | Opaque lifecycle ports and exact transition validation | S02 |
| K02 | `2026-07-15-k02-kernel-authority-cleanup.md` | Hierarchical budgets, deterministic cleanup and removal of Executive-local kernel | K01 |
| X01 | `2026-07-15-x01-executive-use-case-ports.md` | Narrow handler ports and extracted context/session/projection services | S02 |
| X02 | `2026-07-15-x02-private-composition-root.md` | Private composition root and split lifecycle bootstrap | X01, K01 |
| F01 | `2026-07-15-f01-domain-facade-authority.md` | Metacog, Cognit and remaining Corpus production paths use authoritative facades | X02, E03 |
| R01 | `2026-07-15-r01-canonical-event-spine.md` | EnvelopeV2, ordered tree sequence and raw observation separation | S02 |
| R02 | `2026-07-15-r02-deterministic-event-projections.md` | Replayable public/debug/memory/Agent/metrics reducers | R01 |
| Q01 | `2026-07-15-q01-layered-config-extension-catalog.md` | Provenanced config schema and policy-scoped extension catalog | X02, E03 |
| Q02 | `2026-07-15-q02-typed-interact-thin-bin.md` | Typed reducer-driven Interact and host-only Bin | Q01, R02 |

The result is 42 implementation plans. The count is intentionally larger than the initial estimate because the original Kernel, composition-root, event-spine and Interact migrations, plus memory lifecycle and SubAgent recovery, cannot remain verifiable when silently combined.

## 6. Per-plan document contract

Every implementation plan starts with:

```markdown
# <Feature> Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** <one externally verifiable result>

**Architecture:** <bounded approach and compatibility strategy>

**Tech Stack:** Rust, Tokio, SQLite/rusqlite, existing Fabric contracts, existing test harnesses

**Prerequisites:** <plan IDs and exact completion evidence>

**Source requirements:** <source-plan path:line anchors>

---
```

Each plan then contains:

1. current-code anchors;
2. invariants and explicit non-goals;
3. exact file map;
4. TDD tasks with full code snippets;
5. per-task focused command and expected failure/pass;
6. compatibility deletion gate;
7. scoped verification;
8. workspace verification;
9. commit message with problem/solution context;
10. completion evidence checklist.

## 7. Generation batches

Generating all plans in one unreviewed pass would recreate the same context and consistency problem. Generate them in ordered batches:

```text
Batch P0: E01-E03, S01-S02
Batch P1: M01-M03, G01-G04, D01-D03, A01, K01, X01, R01
Batch P2: K02, X02, R02, A02-A03, C01, M04-M07, G05-G08, Q01
Batch P3: M08, G09-G10, F01, Q02, C02, V01-V02
```

After each batch:

- re-read all four source designs;
- re-grep every referenced production symbol;
- run a cross-plan type/signature consistency review;
- ensure no later plan is silently required by an earlier plan;
- commit the batch as documentation only.

## 8. Completion criteria for decomposition

The decomposition is complete only when:

- all 42 plan files exist;
- every source phase maps to at least one plan and task;
- every plan uses verified current paths and symbols;
- no placeholder such as `TBD`, `TODO`, “implement later” or “similar to” remains;
- shared type signatures are identical across every consuming plan;
- the DAG has no unresolved cycle;
- each plan has a runnable focused test command and full workspace gate;
- each plan ends in deployable behavior or an isolated unused contract;
- V02 proves the final integrated production behavior rather than only unit tests.
