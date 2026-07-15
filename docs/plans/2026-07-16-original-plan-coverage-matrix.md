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
| Opaque Kernel runtime, exact transitions and parent/owner validation | `architecture-coupling-optimization-plan.md:1012-1017`, `:1027-1031` | K01 | Runtime: `crates/kernel/src/runtime.rs:18`; total matrices: `crates/fabric/src/types/process.rs:71`, `crates/fabric/src/types/operation.rs:61`; orphan/ownership tests: `crates/kernel/tests/lifecycle_integrity.rs:22`, `:31`, `:60`; opaque runtime/cleanup tests: `crates/kernel/tests/kernel_runtime.rs:7`, `:28`, `:47` | done |
| Kernel becomes the sole lifecycle authority and Executive-local kernel is deleted | `architecture-coupling-optimization-plan.md:1013-1025`, `:1029-1032` | K02 | Opaque authority: `crates/kernel/src/runtime.rs:24-47`; private tables: `crates/kernel/src/process/mod.rs:4-6`, `crates/kernel/src/operation/mod.rs:3-6`, `crates/kernel/src/space/mod.rs:3-4`; split composition: `crates/executive/src/core/domain_ports.rs:6-18` and `crates/executive/src/impl/daemon/bootstrap/mod.rs:18-35`; deletion gates: `scripts/architecture-check.sh:74-124`; deterministic scenarios: `crates/executive/tests/kernel_lifecycle_scenarios.rs:145-260`, `crates/kernel/tests/terminal_cleanup.rs`, `crates/kernel/tests/hierarchical_budget.rs` | done |
| Give request handlers only use-case ports and extract context/session/projection flows | `architecture-coupling-optimization-plan.md:1038-1054`, `:1058-1063` | X01 | `HandlerPorts`: `crates/executive/src/impl/daemon/handler/ports.rs:20`; request deletion gate: `crates/executive/tests/request_use_case_boundaries.rs:17`; workspace validation recorded in `2026-07-15-x01-executive-use-case-ports.md:146-161` | done |
| Delete `CoreSystems` and split private lifecycle bootstrap | `architecture-coupling-optimization-plan.md:1055-1056`, `:1062-1063` | X02 | Private root: `crates/executive/src/impl/daemon/bootstrap/mod.rs:18-35`; thin handler init: `crates/executive/src/impl/daemon/handler/init.rs:1-41`; deletion/confinement gate: `crates/executive/tests/private_composition_root.rs:17-105`; workspace and architecture evidence: `2026-07-15-x02-private-composition-root.md` completion record | done |
| Authoritative domain facades | `architecture-coupling-optimization-plan.md:1065-1115` | M01-M08, D01-D03, A01-A03, G01-G10, F01 | Memory/conscious/SubAgent slices mapped; Metacog/Cognit/Corpus cleanup requires F01 | mapped |
| Canonical event spine and deterministic projections | `architecture-coupling-optimization-plan.md:1117-1139` | R01, R02 | Detailed plans still required | mapped |
| Layered config, extension catalog, typed Interact client and thin Bin | `architecture-coupling-optimization-plan.md:1141-1160` | Q01, Q02 | Detailed plans still required | mapped |
| Optional physical crate split only after stable boundaries | `architecture-coupling-optimization-plan.md:1162-1170` | V02 decision record | Explicitly optional; V02 records evidence and decision | mapped |

## Conscious-core coverage

| Original requirement | Source anchor | Executable owner | Status |
|---|---|---|---|
| Configured temporality and event-driven restartable Sorge lifecycle | `dasein-agora-conscious-core-plan.md:789-794`, `:803-804` | D01 | done |
| Versioned Dasein reducer and complete structured event handling | `dasein-agora-conscious-core-plan.md:786-791`, `:799-803` | D02 | done |
| Self ledger, checksums, replay and causal lineage | `dasein-agora-conscious-core-plan.md:786-804` | D03 | done |
| Agora transaction and durability integrity | `dasein-agora-conscious-core-plan.md:811-814`, `:819-824` | A01 | done |
| Typed workspace contents with provenance, visibility and lifecycle | `dasein-agora-conscious-core-plan.md:810`, `:824-825` | A02 | done |
| Scoped Scratchpad integration and direct-mutation cleanup | `dasein-agora-conscious-core-plan.md:815-817` | F01 | mapped |
| Bounded typed competition, deterministic selection and ignition | `dasein-agora-conscious-core-plan.md:827-833`, `:836-843` | A02 | done |
| Durable broadcast epochs, delivery and acknowledgements | `dasein-agora-conscious-core-plan.md:834-835`, `:844-845` | A03 | Durable store/coordinator: `crates/agora/src/broadcast/store.rs:18`, `crates/agora/src/broadcast/mod.rs:193`; replay test: `crates/agora/tests/broadcast_delivery.rs:84`; bounded visibility/terminal ACK test: `crates/agora/tests/broadcast_delivery.rs:220`; restart/finalization test: `crates/agora/tests/broadcast_delivery.rs:322`; non-leaking contract test: `crates/fabric/tests/workspace_broadcast_contract.rs:105` | done |
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
