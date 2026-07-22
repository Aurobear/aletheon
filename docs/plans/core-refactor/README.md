# Core Architecture Refactor Plan Set

This directory is the complete execution package for implementing
`docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` with DeepSeek or another implementation agent.

## Start here

1. Read `00_DEEPSEEK_EXECUTION_GUIDE.md`.
2. Execute `01_PHASE_0_ARCHITECTURE_BASELINE_AND_GATES.md` first.
3. Do not start a later phase until its hard prerequisites are committed and its required Phase 0 inventories contain the affected surfaces.
4. Give the implementation agent only one phase document plus the architecture design and current repository instructions at a time.
5. Require the report format in the execution guide after every task/commit.

## Progress tracker

- [x] Phase 0 — architecture baseline and gates
- [x] Phase 1 — Fabric contract purification
- [x] Phase 2 — Executive layering
- [ ] Phase 3 — coding runtime decoupling
- [ ] Phase 4 — configuration ownership
- [ ] Phase 5 — supplemental memory generalization
- [ ] Phase 6 — channel, identity, and information sources
- [ ] Phase 7 — inference adapter isolation
- [ ] Phase 8a — Agent Control state machine
- [ ] Phase 8b — Turn Pipeline state machine
- [ ] Phase 8c — Mnemosyne service state machine
- [ ] Phase 8d — MCP client/auth state machine
- [ ] Phase 8e — daemon server state machine
- [ ] Phase 9 — public API contraction
- [ ] Phase 10 — global verification and crate-split review

## Documents

| File | Deliverable |
|---|---|
| `00_DEEPSEEK_EXECUTION_GUIDE.md` | execution order, reports, stop rules, commits |
| `01_PHASE_0_ARCHITECTURE_BASELINE_AND_GATES.md` | inventories, metrics, fixtures, CI ratchets |
| `02_PHASE_1_FABRIC_CONTRACT_PURIFICATION.md` | neutral identity/source/event contracts |
| `03_PHASE_2_EXECUTIVE_LAYERING.md` | application/adapter/composition/host separation |
| `04_PHASE_3_CODING_RUNTIME_DECOUPLING.md` | neutral runtime contracts and private Pi adapter |
| `05_PHASE_4_CONFIG_OWNERSHIP.md` | deployment/normalized/domain/adapter config pipeline |
| `06_PHASE_5_SUPPLEMENTAL_MEMORY.md` | product-neutral supplemental memory |
| `07_PHASE_6_CHANNEL_IDENTITY_SOURCES.md` | neutral channel/identity/source ports and adapters |
| `08_PHASE_7_INFERENCE_ADAPTERS.md` | private provider adapters and capability routing |
| `09_PHASE_8_STATE_MACHINES.md` | five independently scheduled state-machine packages |
| `10_PHASE_9_PUBLIC_API_CONTRACTION.md` | stable facades and implementation-tree closure |
| `11_PHASE_10_GLOBAL_VERIFICATION.md` | completion audit, serial workspace checks, split decision |

## Authority order

When documents disagree, stop and resolve them in this order:

```text
current code and persisted/wire evidence
    > docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md
    > the current phase plan
    > older plans or comments
```

A disagreement about current behavior must be reported rather than silently resolved by the implementation agent.
