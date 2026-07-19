# July 2026 Architecture Plan Completion Ledger

> Status: Completed-work record
>
> Audited: 2026-07-17 against the current branch

This record replaces completed or superseded implementation plans removed from
`docs/plans`. Git history remains the source for their original task text. An
item is recorded as complete only when its production boundary and deterministic
acceptance evidence are present. Active and partial plans remain indexed in
`docs/plans/2026-07-16-original-plan-coverage-matrix.md`.

## Completed architecture and runtime slices

| ID | Result | Current evidence |
|---|---|---|
| E02 | Complete | Governed execution and fail-closed permit validation: `crates/corpus/src/tools/capability_executor.rs:118-186`. |
| E03 | Complete | Single governed turn surface and shared graph: `crates/executive/src/service/governed_capability.rs:103-216`. |
| F01 | Reopened / Partial | The static gate exists at `crates/executive/tests/domain_facade_authority.rs:20-76`, but production services still retain concrete domain state in `crates/executive/src/service/request_use_cases.rs:70-101`, `crates/executive/src/service/exec_session.rs:16-23`, and `crates/executive/src/service/turn_runtime_ports.rs:105-128`. The active implementation contract is restored at `docs/plans/2026-07-15-f01-domain-facade-authority.md`. |
| Q01 | Complete | Executive configuration projection: `crates/executive/src/core/config/mod.rs:37-68`; scoped extension service: `crates/executive/src/service/extension_service.rs:98-150`. |
| R01 | Complete | Append-only event spine: `crates/fabric/src/events/spine.rs:154-166`; repository acceptance: `crates/executive/tests/event_spine_repository.rs:1-190`. |
| R02 | Complete | Projection restart and poison isolation: `crates/executive/tests/event_projection_contract.rs:84-170`; byte-stable replay: `crates/executive/tests/event_projection_replay.rs:1-80`. |
| S01 | Complete | Canonical store: `crates/executive/src/impl/session/canonical_store.rs:13`; append/reopen/fork acceptance: `crates/executive/tests/session_append_store.rs:27-182`. |
| X01 | Complete | Handler use-case ports: `crates/executive/src/impl/daemon/handler/ports.rs:21-54`; boundary checks: `crates/executive/tests/request_use_case_boundaries.rs:17-112`. |
| X02 | Complete | Private composition root: `crates/executive/src/impl/daemon/bootstrap/mod.rs:18-35`; confinement checks: `crates/executive/tests/private_composition_root.rs:17-108`. |
| E01 | Complete | Shrink-only architecture gates: `scripts/architecture-check.sh:37-160`; Bin now depends only on Executive and Interact domain surfaces: `crates/bin/Cargo.toml:16-23`. |
| Q02 | Complete | Deterministic typed reducer/reconnect handling: `crates/interact/tests/tui_reducer.rs:20-107`; thin host routing: `crates/bin/tests/host_routing.rs:3-26`. |
| S02 | Complete | Unified lifecycle and canonical replay acceptance: `crates/executive/tests/turn_coordinator_lifecycle.rs:40-167`; the required architecture gate now passes. |
| V01 | Complete | Deterministic replay/authority evidence: `crates/executive/tests/cross_domain_acceptance.rs:20-93`; reproducible indicators and ablations: `crates/executive/tests/functional_indicators.rs:73-140`. |

## Completed conscious-core slices

| ID | Result | Current evidence |
|---|---|---|
| A01 | Complete | Bound permit contract: `crates/fabric/src/include/agora.rs:233-277`; commit-time version validation: `crates/agora/src/workspace/mod.rs:130-187`. |
| A02 | Complete | Typed workspace contracts: `crates/fabric/src/types/workspace.rs:208-431`; bounded deterministic competition: `crates/agora/src/competition/mod.rs:11-331`. |
| A03 | Complete | Durable broadcast lifecycle: `crates/agora/src/broadcast/mod.rs:200-230`. |
| C01 | Complete | Recurrent coordination path: `crates/executive/src/service/conscious_core_coordinator.rs:291-373`; causal acceptance: `crates/executive/tests/conscious_core_recurrence.rs:254-389`. |
| C02 | Complete | Bounded processors: `crates/executive/src/service/conscious_workspace.rs:176-257`; read-only inspector: `crates/executive/src/service/conscious_core_inspector.rs:20-86`. |
| D01 | Complete | Validated configuration and injected lifecycle: `crates/dasein/src/dasein/mod.rs:61-137`, `crates/dasein/src/dasein/sorge.rs:12-21`. |
| D02 | Complete | Versioned reducer contracts: `crates/fabric/src/dasein/transition.rs:80-233`, `crates/dasein/src/dasein/reducer.rs:28-241`. |
| D03 | Complete | Durable lineage and replay: `crates/fabric/src/dasein/transition.rs:238-286`, `crates/dasein/src/dasein/ledger.rs:187-249`. |

## Completed memory slices

| ID | Result | Current evidence |
|---|---|---|
| M02 | Complete | Canonical records and scopes: `crates/mnemosyne/src/model/record.rs:8-203`, `crates/mnemosyne/src/model/scope.rs:5-68`. |
| M03 | Complete | Unified local recall and degradation: `crates/mnemosyne/src/service.rs:459-526`; active contract tests: `crates/mnemosyne/tests/unified_memory_contract.rs:73-282`. |
| M04 | Complete | Bounded projection: `crates/mnemosyne/src/projection.rs:78-168`; production processor: `crates/executive/src/impl/conscious/memory_processor.rs:65-102`. |
| M07 | Complete | Governed tombstone/retention transaction: `crates/mnemosyne/src/retention/repository.rs:80-181`; management entry: `crates/executive/src/service/request_use_cases.rs:884-910`. |
| M08 | Complete | Trusted child scope: `crates/mnemosyne/src/agent_scope.rs:115-184`; reviewed promotion: `crates/mnemosyne/src/promotion.rs:27-110`. |
| M05 | Complete | Lifecycle-aware leased extraction: `crates/mnemosyne/src/service.rs:370-407,711-723`; deterministic insert/merge/reject/supersede: `crates/mnemosyne/src/consolidation/consolidator.rs:101-146`. |
| M06 | Complete | Mnemosyne-owned receipt/retry/tombstone settlement: `crates/mnemosyne/src/backends/gbrain/reconcile.rs:132-276`; Executive is scheduling-only: `crates/executive/src/impl/gbrain/worker.rs:14-67`. |
| Mnemosyne unified memory | Complete | Sanitized closed-label observability: `crates/mnemosyne/src/observability.rs:191-220`; production consolidation and GBrain ownership are covered by M05/M06 above. |

## Completed Agent and Kernel slices

| ID | Result | Current evidence |
|---|---|---|
| G02 | Complete | Typed control contracts: `crates/fabric/src/types/agent_control.rs:126-216`; contract acceptance: `crates/fabric/tests/agent_control_contract.rs:64-187`. |
| G03 | Complete | Transactional control service: `crates/executive/src/service/agent_control/mod.rs:572-1021`; persistent CAS: `crates/executive/src/service/agent_control/sqlite_repository.rs:120-235`. |
| G04 | Complete | Native Cognit execution: `crates/executive/src/impl/runtime/native_cognit.rs:318-370,440-468`; acceptance: `crates/executive/tests/native_cognit_runtime.rs:229-568`. |
| G05 | Complete | Trusted Agent tools: `crates/corpus/src/tools/tools/agent_control.rs:105-109,187-266`; schema and identity tests: `crates/corpus/tests/agent_control_tools.rs:157-278`. |
| G06 | Complete | Bounded context fork: `crates/executive/src/service/agent_control/context_fork.rs:97-214`; projection acceptance: `crates/executive/tests/agent_agora_projection.rs:325-415`. |
| G07 | Complete | Durable priority mailbox: `crates/executive/src/service/agent_control/mod.rs:878-978`; restart/overload acceptance: `crates/executive/tests/agent_mailbox.rs:75-219`. |
| G09 | Complete | Child memory boundary: `crates/executive/src/service/agent_control/memory.rs:29-76`; reviewed promotion lineage: `crates/mnemosyne/src/promotion.rs:27-126`. |
| G10 | Complete | Restart decisions: `crates/executive/src/service/agent_control/recovery.rs:60-155`; verified cleanup: `crates/executive/src/service/agent_control/cleanup.rs:37-82`. |
| K01 | Complete | Exact lifecycle matrices: `crates/fabric/src/types/process.rs:85-105`, `crates/fabric/src/types/operation.rs:62-81`; opaque runtime: `crates/kernel/src/runtime.rs:28-52`. |
| K02 | Complete | Terminal cleanup: `crates/kernel/src/runtime.rs:463-535`; hierarchical budget contract: `crates/fabric/src/types/admission.rs:115-153`. |
| G08 | Complete | FIFO bounded waiting and admission: `crates/executive/src/service/agent_control/admission.rs:183-270`; exactly-once settlement/revocation: `crates/executive/src/service/agent_control/admission.rs:375-465`. |
| SubAgent unified harness | Complete | Pinned Pi build/protocol identity: `crates/executive/src/impl/runtime/pi.rs:45-55`; strict fail-closed JSONL lifecycle parsing: `crates/executive/src/impl/runtime/pi_protocol.rs:19-149`. |

## Completed session-recovery repair

The session compaction design and implementation plan are complete: UTF-8-safe
bounding is at `crates/fabric/src/include/compaction.rs:22-32,118-139`, transient
tool-output shaping at `crates/cognit/src/harness/linear/tool_output.rs:3-46`,
tail preservation at `crates/mnemosyne/src/impl/compressor/tail.rs:20-118`, and
fallible compaction propagation at
`crates/executive/src/impl/daemon/session_manager.rs:189-223`.

## Superseded planning slices

| ID | Reason |
|---|---|
| P0 | Its readiness slices have concrete successors; the stale checklist is no longer an execution authority. |
| G01 | The original spawner baseline was replaced by the G04/G05 control/runtime boundary; compatibility remains tested at `crates/executive/tests/subagent_production_baseline.rs:116-154`. |
| M01 | Its ignored-baseline tests were activated by M03; current coverage is `crates/mnemosyne/tests/unified_memory_contract.rs:163-416`. |
