# Original Plans to Executable Work Coverage Matrix

> **For agentic workers:** This is the traceability authority for the post-P0 implementation. A row is complete only when its executable plan, production code, focused tests, workspace tests and deletion gate are all evidenced.

**Goal:** Preserve every required outcome from the four original plans while turning them into dependency-ordered, reviewable implementation slices.

**Source plans:**

- `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md`
- `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md`
- `docs/plans/2026-07-15-subagent-unified-harness-plan.md`

## Status vocabulary

| Status | Evidence required |
|---|---|
| `done` | Merged code and tests satisfy the source acceptance criteria. |
| `planned` | A dedicated executable plan exists with exact paths, TDD steps and deletion gates. |
| `mapped` | An execution ID and dependency position exist, but the detailed plan is not yet written. |
| `missing` | No executable owner exists; implementation must not start until corrected. |

## Architecture plan coverage

| Original requirement | Source anchor | Executable owner | Current evidence | Status |
|---|---|---|---|---|
| Freeze dependency and bypass drift | `architecture-coupling-optimization-plan.md:937-956` | E01 | `scripts/architecture_check.py`, CI gate and shrink-only baseline | done |
| One governed capability path | `architecture-coupling-optimization-plan.md:958-979` | E02, E03 | Corpus executor plus Executive governed invoker | done |
| Canonical Session/Turn/Item lifecycle | `architecture-coupling-optimization-plan.md:981-1006` | S01, S02 | canonical store and shared `TurnCoordinator` | done |
| Kernel is the sole lifecycle authority | `architecture-coupling-optimization-plan.md:1008-1032` | K01, K02 | Detailed plans still required | mapped |
| Replace `CoreSystems` god container with use-case ports | `architecture-coupling-optimization-plan.md:1034-1063` | X01, X02 | Detailed plans still required | mapped |
| Authoritative domain facades | `architecture-coupling-optimization-plan.md:1065-1115` | M01-M08, D01-D03, A01-A03, G01-G10, F01 | Memory/conscious/SubAgent slices mapped; Metacog/Cognit/Corpus cleanup requires F01 | mapped |
| Canonical event spine and deterministic projections | `architecture-coupling-optimization-plan.md:1117-1139` | R01, R02 | Detailed plans still required | mapped |
| Layered config, extension catalog, typed Interact client and thin Bin | `architecture-coupling-optimization-plan.md:1141-1160` | Q01, Q02 | Detailed plans still required | mapped |
| Optional physical crate split only after stable boundaries | `architecture-coupling-optimization-plan.md:1162-1170` | V02 decision record | Explicitly optional; V02 records evidence and decision | mapped |

## Conscious-core coverage

| Original requirement | Source anchor | Executable owner | Status |
|---|---|---|---|
| Configured temporality and event-driven restartable Sorge lifecycle | `dasein-agora-conscious-core-plan.md:789-794`, `:803-804` | D01 | done |
| Versioned Dasein reducer and complete structured event handling | `dasein-agora-conscious-core-plan.md:786-791`, `:799-803` | D02 | mapped |
| Self ledger, checksums, replay and causal lineage | `dasein-agora-conscious-core-plan.md:786-804` | D03 | mapped |
| Typed Agora transactions and durable integrity | `dasein-agora-conscious-core-plan.md:806-825` | A01 | mapped |
| Bounded competition, ignition, epochs and delivery | `dasein-agora-conscious-core-plan.md:827-845` | A02, A03 | mapped |
| Dasein–Agora recurrent loop | `dasein-agora-conscious-core-plan.md:847-865` | C01 | mapped |
| Memory, Metacog, Corpus and child processors | `dasein-agora-conscious-core-plan.md:867-884` | C02 | mapped |
| Functional indicators and ablations | `dasein-agora-conscious-core-plan.md:703-747` | V01 | mapped |

## Mnemosyne coverage

The former six-row decomposition compressed two independently testable source phases. They are restored as M07 and M08 so retention and child-memory isolation cannot disappear inside broad integration work.

| Original phase | Source anchor | Executable owner | Status |
|---|---|---|---|
| M1 behavior baseline | `mnemosyne-unified-memory-plan.md:302-326` | M01 | done |
| M2 canonical record and scope | `mnemosyne-unified-memory-plan.md:328-353` | M02 | done |
| M3 unified local recall | `mnemosyne-unified-memory-plan.md:355-395` | M03 | done |
| M4 bounded conscious-core projection | `mnemosyne-unified-memory-plan.md:397-451` | M04 | mapped |
| M5 leased extraction and consolidation | `mnemosyne-unified-memory-plan.md:453-502` | M05 | mapped |
| M6 GBrain reconciliation | `mnemosyne-unified-memory-plan.md:504-529` | M06 | mapped |
| M7 retention and forgetting | `mnemosyne-unified-memory-plan.md:531-549` | M07 | mapped |
| M8 multi-Agent isolation and promotion | `mnemosyne-unified-memory-plan.md:551-575` | M08 | mapped |

## SubAgent coverage

The former seven-row decomposition combined mailbox, budgets, memory promotion and recovery. These have distinct authorities and failure modes, so the original ten phases are retained one-to-one as G01-G10.

| Original phase | Source anchor | Executable owner | Status |
|---|---|---|---|
| A1 production baseline | `subagent-unified-harness-plan.md:402-426` | G01 | done |
| A2 shared control contracts | `subagent-unified-harness-plan.md:428-451` | G02 | done |
| A3 transactional control service | `subagent-unified-harness-plan.md:453-506` | G03 | mapped |
| A4 Native Cognit runtime | `subagent-unified-harness-plan.md:508-549` | G04 | mapped |
| A5 thin Agent tools | `subagent-unified-harness-plan.md:551-584` | G05 | mapped |
| A6 bounded context and Agora projection | `subagent-unified-harness-plan.md:586-619` | G06 | mapped |
| A7 live mailbox | `subagent-unified-harness-plan.md:621-640` | G07 | mapped |
| A8 admission and hierarchical budgets | `subagent-unified-harness-plan.md:642-662` | G08 | mapped |
| A9 memory isolation and result promotion | `subagent-unified-harness-plan.md:664-688` | G09 | mapped |
| A10 restart recovery and cleanup | `subagent-unified-harness-plan.md:690-708` | G10 | mapped |

## Corrected post-P0 execution order

```text
S02 (done)
 |
 +--> M01 -> M02 -> M03 ------------------------------+
 +--> G01 -> G02 -> G03 -> G04 -> G05                 |
 +--> D01 -> D02 -> D03 ----+                         |
 +--> A01 -> A02 -> A03 ----+--> C01 --> M04 -> M05 -> M06 -> M07
 +--> K01 -> K02 -----------+     |       |                    |
 +--> X01 -> X02 -----------------+       +--> G09              |
 +--> R01 -> R02 -----------------+                            |
 +--> Q01 -> Q02 -----------------+                            |
                                  +--> G06 -> G07 -> G08 -> G09 -> G10
                                  +--> C02 <--------------------+
                                           |
                              F01 --------> V01 -> V02
```

M08 is implemented with G09 because both describe the same promotion boundary; both IDs remain in acceptance evidence. Optional physical crate splitting is decided—not automatically performed—at V02, exactly as required by the source.

## Completion rule

The project is not complete merely because all rows have code. Completion requires:

1. every `mapped` row becomes `planned`, then `done`;
2. every original acceptance item has at least one deterministic test locator;
3. compatibility adapters list and satisfy an explicit deletion gate;
4. `cargo test --workspace` and architecture fitness checks pass;
5. V02 proves installed-daemon behavior, restart, rollback and bounded failure scenarios.
