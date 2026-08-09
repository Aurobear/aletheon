# Agent Kernel V2：APX-00 Application / I/O census

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §7
State: evidence-only census closed; `application-use-case-census.tsv` frozen; no behavior change

## Context receipt

```text
Slice: APX-00 application use-case + I/O census
Baseline commit: 0bf690b2
Plan revision: implementation-plan §7.1-7.3
Direct prerequisites: RA-00, K0 (frozen authority + effect baselines)
Current authoritative writer: unchanged legacy Executive Application paths
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: enumerated per row (daemon/CLI/RPC/Gmail/Telegram)
Test-only callers: excluded (production_callers=0 convention)
Installed/config callers: cross-checked against config/architecture/persistence-surfaces.tsv
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: per-row closing slice
Unknowns/blockers: optional-unproven rows carry INVESTIGATE closure requirements; none left open
Expected files: config/architecture/application-use-case-census.tsv, this evidence record, gate
Out-of-scope files: all production behavior, writer cutovers, APX-01+
```

## 1. Use-case classification (plan §7.2)

| classification | count | families |
|---|---|---|
| `basic-use-case` | 10 | daemon turn, react, exec single-message, Session create/resume/fork, Approval, Goal Draft + external stimulus, Memory gateway, Embodied execution, Conscious core, Admin |
| `optional-supported` | 3 | Extension activation/install, Governed review, Evaluation vertical |
| `optional-unproven` | 3 | Evolution proposal, Orchestration multi-agent, Capability benchmark |
| `adapter-io` | 6 | workspace trust/checkpoint, storage quota, SQLite concrete I/O, FS concrete I/O, process concrete I/O, network concrete I/O |
| `compatibility` | 2 | V0 command dispatcher, legacy Goal CRUD/status/resume |
| `dead` | 2 | stale interact tui goal/workflow debug shells |

### optional-unproven closure requirements (§7.2)

- `APX-UC-13` Evolution proposal — production caller `GovernedEvolutionProposer` composed at `post_turn_projection.rs`; installed evidence: search `evolution-proposals.db`; target owner `Metacog (propose/experiment only)`; closing slice `APX-01/D3`.
- `APX-UC-14` Orchestration — production caller orchestration runtime path; installed evidence: search installed orchestration config; target `Application AgentSupervisor`; closing slice `RA-05`.
- `APX-UC-17` Capability benchmark — production caller `CapabilityRollupProjectionSink::open` (`capability_benchmark.rs`); installed evidence `evaluation-rollups.db`; target `Application Evaluation`; closing slice `APX-03`.

None are silently rebuilt into a new Application; each has a target owner and closing slice.

## 2. Concrete I/O in the Application layer (plan §7.1)

`APX-IO-01..04` enumerate concrete SQLite/filesystem/process/network I/O inside `application/**`:

- **SQLite**: `goal/store.rs` (`INSERT INTO objectives` :119), `approval/repository.rs` (`Connection::open` :108,126), `governed_review/store.rs`, `workspace_trust.rs`, `evaluation/`, `orchestration/store.rs` — all owned by daemon bootstrap today.
- **Filesystem**: `extension_install.rs:240` (`std::fs::write`), `extension_snapshot.rs:179`, `harness_factory.rs`.
- **Process**: `goal/verification.rs`, `goal/worker.rs`, `agent_control/settlement.rs`, `embodied_execution_adapter.rs` — `Command::new`.
- **Network**: `goal/attempt.rs`, `extension_coordinator.rs` — `reqwest`/`tokio::net`.

All are target owners for `APX-04` (SQLite/FS/process/host adapters out of Application core) and `RA-05` (process), matching the plan's "每张表只有一个写 owner" rule.

## 3. Compatibility / dead disposition

- `APX-UC-18` V0 command dispatcher (`prompt_completion_from_rpc_result`, command_dispatcher.rs:19) — pure schema conversion, `CGP-06/XRET-04` deletion after typed Application result.
- `APX-UC-19` legacy `GoalService` — `goal_service.rs:74`, `APX-03/XRET-04`.
- `APX-DEAD-01/02` — stale interact `tui/goal.rs`, `debug.rs`, `rpc_client.rs`, `workflow.rs` — no production caller, marked `INVESTIGATE` (`I9-OPTIONAL`) per census ledger; DELETE only with caller evidence.

## 4. APX-01 precondition (plan §7.3)

`APX-01` may only create the minimal Application surface once basic use cases, Approval, GoalDraft/external stimulus, and real supported optional packages are settled. This census closes the classification; `APX-01` remains gated on the review of this artifact.

## 5. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS (23 findings, 0 deps)
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p executive --lib  PASS (4.30s)
```

### Gate mutation tests (probes, reverted after each)

| probe | result |
|---|---|
| new `rusqlite::Connection::open` in `world_state.rs` (uncensed) | REJECTED (unregistered Application I/O) |
| drop `APX-IO-01` (SQLite family) row | REJECTED (its files become uncovered) |
| drop `APX-UC-06` (goal) row | REJECTED (goal I/O uncovered) |
| structural corruption (wrong col count) | REJECTED |
| clean baseline | ACCEPTED (26 rows, 11 cols, all evidence matches) |

APX-00 gate is strict per-file: every production application file with concrete I/O must be explicitly listed in a census row (UC or IO). No regex auto-coverage.

## 6. Scope boundaries

- **Allowed**: application-use-case-census.tsv, this evidence record, APX-00 gate.
- **Explicitly not done**: no Application surface, no port, no adapter move, no writer cutover, no schema change, no deployment.
