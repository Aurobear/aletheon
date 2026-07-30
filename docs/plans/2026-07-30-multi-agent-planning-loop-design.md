# Multi-Agent Planning Loop (B)

**Date:** 2026-07-30
**Status:** Revised design; activation, authority, ordering, failure, and
cancellation decisions locked; implementation plan pending
**Scope:** Activate the typed multi-agent planning loop so a user turn can be
decomposed into a Planner → Explorer → Executor → Tester → Reviewer role graph,
with bounded Fixer → Tester → Reviewer repair loops when validation or review
fails. Children are spawned, coordinated through Agora, and folded back into the
turn's cognitive contract. Wire the already-implemented-but-dead
`CognitiveRoleWorkflow` into the production turn pipeline behind a typed,
host-owned decomposition policy. Natural-language classification is prohibited.

> Dependency: **B requires E′ (Per-Child Capability Attenuation)** —
> `docs/plans/2026-07-30-per-child-capability-attenuation-design.md`. The role
> children spawned here MUST be attenuated on spawn so that
> `child-effective ⊆ parent-effective`. E′ is Wave 0; B is Wave 1 and MUST NOT
> land before E′ closes the spawn choke point at
> `crates/executive/src/application/agent_control/mod.rs:856`. B also consumes
> E′'s typed grant: every role-child spawn MUST carry the host-only
> `AgentToolContext.delegator_authority` (`AgentDelegationAuthority`) minted at
> the runtime composition sites (`turn_pipeline.rs:699`, `native_cognit.rs:261`)
> from effective runtime state — never reconstructed from profile defaults — and
> B MUST NOT add a second attenuation path (E′ §3.2, §9).

> Roadmap context: the closed-loop role graph was pre-specced (the canonical
> `CognitiveRole` set `Root, Planner, Explorer, Executor, Reviewer, Tester,
> Fixer` at `crates/fabric/src/types/cognitive_workflow.rs:14-22`). This doc
> formalizes and grounds the *wiring*, not the role vocabulary.

## 1. Background & Problem

The planning loop is not one missing piece — it is a **complete typed role graph
that has no production caller**, sitting next to a **wired Agora substrate**.
Three sub-parts, verified:

### 1.1 The toy planner is dead (the named "unwired" item)

`crates/cognit/src/core/planner.rs` defines `Planner` (`planner.rs:17`), a
string/heuristic planner: `generate_plan` (`planner.rs:25`) maps one `Intent`
to one `Plan` with a hard-coded rollback table (`planner.rs:233-250`) and a
keyword risk estimate (`planner.rs:254-266`). Its only reference in the tree is
the re-export `pub use core::planner::Planner;` at
`crates/cognit/src/lib.rs:32`. Its sibling `Reasoner`
(`crates/cognit/src/core/reasoner.rs:25`) is in the same state. **No production
caller constructs either.** This violates architecture discipline §8 ("不创建没有
生产 caller 的 crate", `docs/design/architecture-overview.md:138`).

### 1.2 The real role graph is implemented but dead

`crates/executive/src/application/cognitive_role_workflow.rs` already implements
the full graph over AgentControl + Agora:

- `CognitiveRoleWorkflow` (`cognitive_role_workflow.rs:215`), constructor
  `new(workspace, invoker)` (`:221`).
- `run_planner_explorer_executor` (`:228-361`) drives the roles
  `[Planner, Explorer, Executor]` (`:236-240`), wrapping each terminal output in
  a `CognitiveArtifactEnvelope` (`:297-305`) and advancing the shared task only
  on host validation (`Plan` at `:835`, `Investigation` at `:868`, `ChangeSet`
  at `:884`).
- `run_reviewer_tester_fixer` (`:388-679`) starts with `Tester → Reviewer`; a
  failed validation or unresolved review drives a bounded
  `Fixer → Tester → Reviewer` repair cycle with preserved finding IDs.
- `run_full_coding_workflow` (`:363-386`) chains both into one receipt
  `FullCodingWorkflowReceipt` (`:203-206`).
- The spawn is done by `AgentControlRoleInvoker` (`:29-63`), whose `invoke`
  (`:84-157`) calls `AgentControlPort::spawn(AgentSpawnRequest{…})` (`:101-122`)
  then `wait` (`:127-133`), requires a `Succeeded` terminal (`:138-143`), and
  parses a strict `CognitiveRoleOutput` (`:147-151`).

**Verified dead:** the only constructions of `CognitiveRoleWorkflow::new` are
tests (`cognitive_role_workflow.rs:1300, :1412, :1461`, all under the
`#[cfg(test)]` block at `:1031`). `crates/executive/src/application/mod.rs:10`
merely declares the module. No daemon/turn path invokes any `run_*`.

### 1.3 The Agora substrate is already wired

Unlike the role graph, its coordination substrate is live:

- `CognitiveWorkspaceCoordinator` (`cognitive_workspace.rs`, `new(agora)` at
  `:97`) is registered as the cognitive-task admission port at bootstrap:
  `AgentControlService::new(…)` (`services.rs:86`) `.with_cognitive_task_admission(
  CognitiveWorkspaceCoordinator::new(agora))` (`services.rs:101-102`).
- The turn pipeline currently **creates at most one Root cognitive task per
  non-empty thread space, not one per turn**:
  `TurnPipeline::run` (`turn_pipeline.rs:337`, returns
  `anyhow::Result<serde_json::Value>`) calls `ensure_root_cognitive_task`, which
  guards on Agora presence (`turn_pipeline.rs:183`), lists tasks
  (`:191`), returns when any task already exists (`:192-194`), and otherwise
  commits a `CognitiveTaskNode{ role: Root, stage: Contract }` (`:196-228`). B
  must replace this reuse rule with a turn-scoped root identity.
- The production `AgoraService` impl actually implements task projection:
  `AgoraRegistry` overrides `project_task` (`crates/agora/src/ops/mod.rs:365-372`)
  and `list_tasks` (`:374-385`). The "not implemented" defaults at
  `crates/fabric/src/include/agora.rs:347-354` are only the fallback for other
  backends — the substrate B needs is real.

### 1.4 How a turn satisfies "invoke an agent" today (the ad-hoc path)

The turn contract already carries an `InvokeAgent` obligation
(`RequiredAction::InvokeAgent{ runtime }`,
`crates/cognit/src/core/cognitive_task.rs:28`). But it is satisfied
**model-driven**: `Session::configure_turn_contract`
(`crates/cognit/src/harness/session.rs:331-425`) maps host-authored
`TurnRequirement`s (`crates/fabric/src/include/turn.rs:57-61`) into
`RequiredAction`s (`session.rs:346-362`) and instructs the model to call the
`agent_spawn` tool (`session.rs:468-469`); the loop detects the tool result at
`crates/cognit/src/harness/linear/tool_exec.rs:516-535` and records terminal
evidence. There is **no typed Planner/Reviewer/Tester graph, no Agora task
coordination, and no host-driven decomposition** on this path.

### 1.5 The gap, precisely

| Piece | State | Anchor |
|---|---|---|
| Toy `Planner`/`Reasoner` | dead, no caller | `planner.rs:17`, `reasoner.rs:25`, `lib.rs:32` |
| `CognitiveRoleWorkflow` role graph | implemented, dead | `cognitive_role_workflow.rs:215`, tests-only `:1300` |
| `AgentControlRoleInvoker` spawn+wait | implemented, dead | `cognitive_role_workflow.rs:84-157` |
| Agora task substrate | wired + implemented | `services.rs:101`, `ops/mod.rs:365` |
| Root cognitive task per turn | wired | `turn_pipeline.rs:196-228` |
| IntentInterpreter | **does not exist** | (grep: no symbol in `crates/`) |
| Production caller: turn → role graph | **missing** | — |
| Receipt → turn contract fold-back | **missing** | — |
| Child capability attenuation on spawn | **missing (E′)** | `agent_control/mod.rs:856` |

B closes rows 6–8 (and depends on row 9 = E′). B does **not** re-invent rows 2–5.

## 2. Goals / Non-goals

**Goals**

1. Introduce a typed `TaskDecompositionPolicy` that maps host state to a
   `TaskDecomposition` — either `Simple` (unchanged single-agent turn) or
   `RoleGraph` (drive `CognitiveRoleWorkflow`). Inputs are explicit task kind,
   explicit `TurnRequirement`, effective configuration, substrate availability,
   and budgets. Objective text is workflow data, never routing input
   (`turn.rs:53-55`).
2. Add the **single production caller** that constructs
   `AgentControlRoleInvoker` + `CognitiveRoleWorkflow` and invokes
   `run_full_coding_workflow` from the turn pipeline, after the Root cognitive
   task exists (`turn_pipeline.rs:225`).
3. Add `cognit::RequiredAction::RunRoleGraph` and fold the
   `FullCodingWorkflowReceipt` (`cognitive_role_workflow.rs:203-206`) into that
   obligation. A RoleGraph turn is complete only when required artifacts and the
   terminal graph receipt are durably committed.
4. Spawn every role child **attenuated** (E′): the child's effective tools,
   workspace, and budget are the intersection with the parent's effective grant.
5. **Single authority**: `CognitiveRoleWorkflow` (over AgentControl + Agora) is
   the sole planning-loop authority. The toy `cognit::Planner` does **not**
   become a second authority.

**Non-goals**

- Building E′ (separate doc; prerequisite).
- Parallel/concurrent role execution (Decision D4 defers it).
- A configurable role DAG (Decision D2 defers it; the canonical fixed graph
  ships first).
- Completing `SubAgentRuntime` context propagation for every runtime
  (`PiRuntime` still falls through to the default at
  `crates/executive/src/adapters/runtime/pi.rs:455+`; separate concern).
- New Agora backends or changing the optimistic-concurrency commit protocol
  (`cognitive_workspace.rs` commit path).
- Removing the `agent_spawn` tool path (§1.4) — it stays for model-initiated
  ad-hoc spawns; B adds the host-driven typed path alongside it.

## 3. Design

### 3.1 Component / data flow

```
user turn
  │
  ▼
TurnPipeline::run                                   turn_pipeline.rs:337
  ├─ ensure_root_cognitive_task ─────────────► Agora: CognitiveTaskNode{Root,Contract}
  │                                                  turn_pipeline.rs:196-228
  ├─ TaskDecompositionPolicy::decompose(typed ctx) ─► TaskDecomposition (NEW, cognit)
  │        │
  │        ├─ Simple ─────────────────────────► normal ReActLoop turn (unchanged)
  │        │                                          step.rs:99 / tool_exec.rs:23
  │        └─ RoleGraph{ scope, evidence } ────► drive workflow ▼
  │
  ▼
CognitiveRoleWorkflow::run_full_coding_workflow      cognitive_role_workflow.rs:363
  base path: Planner → Explorer → Executor → Tester → Reviewer
  repair path on failed gate: Fixer → Tester → Reviewer
     AgentControlRoleInvoker::invoke(packet, binding) cognitive_role_workflow.rs:84
        └─ AgentControlPort::spawn(AgentSpawnRequest) cognitive_role_workflow.rs:101
              └─ AgentControlService::spawn            agent_control/mod.rs:856
                    └─ [E′] attenuate child ⊆ parent   (prerequisite)
        └─ wait → CognitiveRoleOutput (strict JSON)    cognitive_role_workflow.rs:127-151
     CognitiveWorkspaceCoordinator                     cognitive_workspace.rs
        .commit_artifact_at(...) + .record_stage_decision_at(...)  (Agora, version-checked)
  │
  ▼
FullCodingWorkflowReceipt ─► fold into CognitiveTurnState obligations ─► turn result
        cognitive_role_workflow.rs:203-206                 cognitive_task.rs (contract)
```

### 3.2 Decision D1 — typed activation policy

| Approach | Summary | Verdict |
|---|---|---|
| D1-A In-loop model classification | Ask the ReAct model whether to decompose | Rejected: prompt-dependent and inside the execution loop. |
| **D1-B Typed host policy** | Explicit requirement or typed coding task plus operator configuration | **Selected:** deterministic, replayable, and host-authoritative. |
| D1-C Pre-turn model classification | Ask another model once and convert its output to an enum | Rejected: a typed output does not make model-derived routing host-authored. |

Add `crates/cognit/src/core/task_decomposition.rs`:

```rust
pub enum TaskDecomposition {
    Simple,
    RoleGraph {
        objective: String,
        workspace_scope: Vec<String>,
        allowed_capabilities: Vec<fabric::AgentRuntimeCapability>,
        expected_evidence: Vec<String>,
    },
}

pub trait TaskDecompositionPolicy: Send + Sync {
    fn decompose(&self, ctx: &DecompositionContext) -> TaskDecomposition;
}
```

`DecompositionContext` contains typed task kind, explicit requirements,
`multi_agent.enabled`, `multi_agent.automatic_for_coding`, Agora availability,
the effective E′ parent grant, and remaining turn budget. It contains no
objective string. Locked decision rules:

1. Explicit `fabric::TurnRequirement::RunRoleGraph` selects `RoleGraph` when E′ and
   Agora are available; a missing prerequisite fails admission.
2. Otherwise, typed `TaskKind::Coding` selects `RoleGraph` only when both config
   flags are enabled.
3. Every other turn selects `Simple`.

The objective is copied into `RoleGraph` only after the typed decision. There is
no model-backed implementation and no natural-language heuristic.

### 3.2.1 Turn-scoped root task

Replace the current "return when any task exists" behavior
(`turn_pipeline.rs:191-194`) with an idempotent root ID derived from the
authoritative turn ID: `root:<turn_id>`. Tasks from earlier turns remain in the
thread space but cannot suppress creation of the current turn's root.

### 3.3 Decision D2 — fixed vs configurable role graph

| Approach | Summary | Trade-off |
|---|---|---|
| **D2-A** Fixed canonical graph (**selected**) | Use the hard-coded `[Planner, Explorer, Executor]` (`:236-240`) + acceptance path beginning `Tester` then `Reviewer`, with conditional `Fixer` repair (`:388-679`) | Zero new surface; matches the canonical `CognitiveRole` set and `CognitiveRoleProfile::canonical` (`cognitive_workflow.rs:103`); single authority. |
| **D2-B** Configurable DAG | Drive the order from `CognitiveTaskNode.dependencies` + `required_artifact_kinds` (`cognitive_workflow.rs:293-309`) | More flexible, but adds a scheduler, cycle checks, and per-turn config; premature for Wave 1. |

**Locked decision: D2-A.** The canonical graph is implemented and host-validated
(`Reviewer` independence is even asserted at
`cognitive_workflow.rs:854`). Model the DAG in the Agora task node now
(the fields already exist) so D2-B is a later, additive change with no rewrite.

### 3.4 Decision D3 — how Agora carries handoffs

| Approach | Summary | Trade-off |
|---|---|---|
| **D3-A** Typed artifacts + StageDecision (**selected**) | Each role terminal → `CognitiveArtifactEnvelope`; only host-validated artifacts advance; gates via `StageDecision` (`Accept/Reject/Replan/Repair/Block`, `cognitive_workflow.rs:525-531`) | Already implemented (`commit_artifact_at` / `record_stage_decision_at` in `cognitive_workspace.rs`); version-checked; auditable; matches the module's stated invariant ("only host-validated artifacts and gate decisions advance", `cognitive_role_workflow.rs:2-3`). |
| **D3-B** Generic blackboard KV | Roles read/write the Agora `Blackboard`/`Workspace` | Untyped, no gate semantics, bypasses validation; regresses the invariant. |
| **D3-C** `task_graph` transitions only | Advance via `TaskGraph::transition` (`task_graph/mod.rs:149`) | Coarse (`Pending/Running/Done/Failed`, `:9-14`); loses artifact provenance. |

**Locked decision: D3-A.** It is the existing mechanism and the only one that
preserves "prose cannot advance the stage" (`cognitive_role_workflow.rs:96`).

### 3.5 Decision D4 — sync vs async role execution

| Approach | Summary | Trade-off |
|---|---|---|
| **D4-A** Sequential (**selected**) | Spawn role → `wait` terminal → gate → next role, as `invoke` already does (`:99-133`) | Deterministic; the version-checked commits (`expected_workspace_version`, `CodingWorkflowRequest:164`) assume serialized advancement; simplest cancellation story. |
| **D4-B** Parallel independent roles | e.g. Explorer ∥ Tester via `JoinSet` + optimistic-version reconciliation | Latency win, but needs conflict handling on `VersionConflict` (`crates/agora/src/workspace/mod.rs`) and a merge policy; defer. |

**Locked decision: D4-A** for Wave 1. Bounded latency is acceptable; correctness
and E′ budget accounting are simpler when roles are serialized.

### 3.6 Types / functions to add or change

- **NEW** `crates/cognit/src/core/task_decomposition.rs`:
  `TaskDecompositionPolicy`, `TaskDecomposition`, `DecompositionContext`, and a
  deterministic typed-state implementation.
  Re-export at `crates/cognit/src/lib.rs` (next to `:32`).
- **CHANGE** `crates/cognit/src/core/planner.rs`: per locked decision O1, mark
  `Planner`/`Reasoner` `#[deprecated]` and drop
  the `lib.rs:32` re-export, since the LLM Planner **role** — not this toy — is
  the planning authority. (No behavior in the loop depends on it today.)
- **NEW** `RoleWorkflowFactory` in Executive. Bootstrap injects stable services
  and canonical role profiles only. For each turn the factory receives root and
  parent IDs, `main_pid`, effective `AgentDelegationAuthority`, workspace, budget
  ledger, and cancellation token, then creates a turn-bound invoker.
- **NEW** production wiring in `crates/executive/src/application/turn_pipeline.rs`:
  a `drive_role_graph(&self, request, decomposition, owner)` method invoked from
  `run` (`:337`) right after `ensure_root_cognitive_task`, that builds
  `CodingWorkflowRequest` and calls
  `CognitiveRoleWorkflow::run_full_coding_workflow`.
- **CHANGE** `crates/executive/src/host/daemon/bootstrap/services.rs`: construct
  the `RoleWorkflowFactory` with AgentControl and a
  `HashMap<CognitiveRole, RoleLaunchProfile>`, and hand the factory to
  `TurnPipeline` (alongside the existing
  `agora` field, `turn_pipeline.rs:47`).
- **CHANGE** `turn_pipeline.rs`: create `root:<turn_id>` and, after the workflow returns,
  `FullCodingWorkflowReceipt`, fold artifact ids / final version into the turn
  result; the durable receipt reference satisfies the corresponding
  `cognit::RequiredAction::RunRoleGraph` obligation.
- **CHANGE** `AgentControlRoleInvoker`: remove daemon-lifetime fixed
  root/parent/workspace identity and accept the immutable turn-bound launch
  context produced by `RoleWorkflowFactory`; reuse its spawn/wait/output parsing.
- **REUSE**:
  (`cognitive_role_workflow.rs:84`), `CognitiveWorkspaceCoordinator`
  (`cognitive_workspace.rs`), `AgoraRegistry::{project_task,list_tasks}`
  (`ops/mod.rs:365,374`), `AgentControlService::spawn` (`agent_control/mod.rs:856`
  — receives already-attenuated inputs from E′).

## 4. Error handling

- **Role runtime failure** — `invoke` already fails closed on a non-`Succeeded`
  terminal (`cognitive_role_workflow.rs:138-143`) or non-JSON output (`:147-151`).
  The workflow records a `StageDecision{ Reject | Replan }` and stops advancing.
  Once any role child has been spawned, `drive_role_graph` never falls back to a
  normal single-agent turn. It persists a partial graph receipt and a terminal
  `Rejected`/`Blocked`/`Cancelled` stage; `RunRoleGraph` remains unsatisfied.
  This prevents duplicate side effects. Automatic activation may choose Simple
  only before the first child spawn when a typed prerequisite disappears;
  explicitly required activation fails instead of downgrading.
- **Planner-role failure / invalid Plan** — the artifact gate rejects at
  validation (`Plan` validated at `:835`); recorded as `Reject`; workflow aborts
  gracefully. The `Fixer` role covers `StageDecisionKind::Repair`
  (`:430`) for downstream fixable findings, not for a missing plan.
- **Agora unavailable** — explicit `RunRoleGraph` fails admission; automatic
  coding activation selects Simple before any child spawn. Mid-workflow, a
  `VersionConflict` from `commit_artifact_at` surfaces as
  `CognitiveWorkspaceError`; policy: re-project the task (fresh
  `expected_workspace_version`) and retry the single commit once, else abort the
  stage with `Block`.
- **Budget / depth limits** — role spawns already set `max_depth: 1`
  (`cognitive_role_workflow.rs:119`), capping recursion; E′ additionally caps
  each budget dimension to the parent minimum. `drive_role_graph` reserves each
  sequential role from one remaining budget ledger; comparing every child with
  the original parent ceiling is insufficient. Total consumed/reserved role
  budget cannot exceed the parent turn budget. The per-role
  wait is bounded by `wait_timeout_ms.min(budget.max_elapsed_ms)` (`:130`).
- **Cancellation** — race `AgentControlPort::wait` against the turn token. On
  cancellation call `AgentControlPort::cancel(root, child)`
  (`fabric/src/types/agent_control.rs:725-729`), then observe the authoritative
  terminal snapshot before persisting `Cancelled`. Async cancellation request
  success is not child terminal success.

## 5. Verification

**Unit tests** — `crates/cognit/src/core/task_decomposition.rs`:

- General task kind remains `Simple` regardless of objective text.
- Coding task kind plus enabled automatic policy returns `RoleGraph`.
- Explicit `RunRoleGraph` fails when Agora or E′ authority is unavailable.
- `RoleGraph` field population maps cleanly onto `CodingWorkflowRequest`.
- A `TurnRequirement::RunRoleGraph` selects `RoleGraph` without a model call.

**Integration tests** — `crates/executive` (extend the existing
`cognitive_role_workflow.rs` test harness at `:1031+` with a wired invoker):

- `role_graph_activation`: a typed coding turn with automatic policy enabled
  drives Planner → Explorer → Executor → Tester → Reviewer; assert one
  `CognitiveArtifactEnvelope` per executed stage is committed, no Fixer is
  spawned when both gates pass, and each `StageDecision` is `Accept`; assert
  `FullCodingWorkflowReceipt` artifact ids are non-empty and the turn result
  references them.
- `agora_none_falls_back`: with `agora: None` (`turn_pipeline.rs:183`), an
  automatically selected coding turn remains Simple and a normal turn returns;
  an explicit `RunRoleGraph` requirement fails admission.
- `role_failure_is_graceful`: a role runtime returning non-`Succeeded` yields a
  `Reject`/`Replan` decision and a partial receipt; it never starts a second
  single-agent execution.
- `turn_cancel_cancels_child`: cancellation calls AgentControl cancel and waits
  for the authoritative terminal snapshot before returning.
- `second_turn_gets_distinct_root`: two turns in one thread create distinct
  `root:<turn_id>` tasks.
- `attenuation_prerequisite` (cross-refs E′): a role child requesting a superset
  of the parent's tools is spawned with the intersection; asserts a
  `CapabilityAttenuated` event (per the E′ doc §4 `AttenuationReport` + §5 spawn ordering, which emits it). This test is the seam where
  B and E′ meet.
- `existing cognitive_role_workflow tests stay green` (`:1300, :1412, :1461`).

**Commands** (via the wrapper, narrowest first):

```
bash scripts/cargo-agent.sh test -p cognit
bash scripts/cargo-agent.sh test -p executive
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

After implementation, the integration owner must run
`sudo bash scripts/aletheon.sh deploy`, prove equal SHA-256 digests for the
release, installed, machine-daemon, and user-daemon executables, observe stable
restart counters, and complete three consecutive real TUI coding runs through
`/usr/bin/aletheon` and the official user socket. At least one unchanged session
must contain multiple turns with distinct turn-scoped roots. Rendered frames,
Agora tasks, child terminal snapshots, Session records, and daemon logs must
agree; development daemons or alternate sockets are diagnostic only.

## 6. Files touched

- `crates/cognit/src/core/task_decomposition.rs` — **new**: typed host policy,
  decomposition types, and unit tests.
- `crates/cognit/src/lib.rs` — re-export the policy; drop/deprecate the
  `Planner` re-export at `:32` (O1).
- `crates/cognit/src/core/planner.rs`, `reasoner.rs` — deprecate per §8 (O1).
- `crates/fabric/src/include/turn.rs:53-61` — add
  `TurnRequirement::RunRoleGraph{…}` as the explicit host-authored activation
  contract.
- `crates/cognit/src/core/cognitive_task.rs:7-42` — add
  `RequiredAction::RunRoleGraph{…}` as the cognitive obligation satisfied only
  by a durable graph receipt.
- `crates/executive/src/application/turn_pipeline.rs` — add `drive_role_graph`,
  call it from `run` (`:337`) after `ensure_root_cognitive_task`; fold the
  receipt into the turn result; hold a `CognitiveRoleWorkflow` handle.
- `crates/executive/src/host/daemon/bootstrap/services.rs` — construct and inject
  a `RoleWorkflowFactory`, never a daemon-lifetime invoker with fixed identity.
- `crates/executive/src/application/cognitive_role_workflow.rs` — add turn-bound
  launch context, remaining-budget ledger, cancellation race, partial receipt,
  and authoritative cancel/wait behavior.
- `crates/executive/src/adapters/runtime/native_cognit.rs:261` and
  `crates/executive/src/application/turn_pipeline.rs:699` — thread E′'s host-only
  `AgentToolContext.delegator_authority` into each role-child spawn, minted from
  effective runtime state (not profile defaults). B adds no attenuation logic of
  its own (E′ §3.2 / §9).
- **Unchanged (reused):** `cognitive_workspace.rs`, `agora/src/ops/mod.rs`,
  `agent_control/mod.rs` (receives attenuated inputs from E′).

## 7. Scope boundary

**B does NOT include:**

- E′ per-child attenuation (Wave 0 prerequisite; `agent_control/mod.rs:856`).
- Parallel role execution (D4-B) or a configurable role DAG (D2-B).
- `SubAgentRuntime` context propagation for runtimes that still fall through to
  the default (`pi.rs:455+`); B only relies on the spawn/wait terminal contract.
- Removing or rewriting the model-driven `agent_spawn` path (§1.4); it coexists.
- Any new Agora backend or commit-protocol change.

**Sequencing:** E′ (Wave 0) → **B (Wave 1)**. A and C are sibling workstreams
that do not block B; B only shares the spawn choke point with E′ and the Agora
substrate with the coding-evaluation kernel
(`docs/plans/2026-07-30-coding-capability-evaluation-kernel-design.md`, which
already exercises `AgentControlService::spawn` from
`capability_benchmark.rs:389-410`). B and the eval kernel must agree on one
`AgentControlRoleInvoker`/spawn convention (O3).

## Locked review decisions

- **O1 — Toy `cognit::Planner`:** deprecate and remove the public re-export;
  the typed Planner role remains the sole planning authority.
- **O2 — Routing:** deterministic typed host state only; no model call and no
  objective-text classifier.
- **O3 — Invoker convention:** B and capability benchmarking use E′'s typed
  `AgentDelegationAuthority` and a shared host launch helper; neither derives
  authority from role/profile defaults.
- **O4 — Receipt encoding:** use `cognit::RequiredAction::RunRoleGraph` with a durable
  `FullCodingWorkflowReceipt` reference.
- **O5 — Cancellation:** cancel the live child through AgentControl and observe
  its terminal snapshot; wait-boundary-only cancellation is insufficient.
