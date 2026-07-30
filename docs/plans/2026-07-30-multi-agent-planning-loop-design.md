# Multi-Agent Planning Loop (B)

**Date:** 2026-07-30
**Status:** Design proposed (decisions recommended; open items flagged for review)
**Scope:** Activate the typed multi-agent planning loop so a user turn can be
decomposed into a Planner → Explorer → Executor → Reviewer → Tester → Fixer role
graph whose children are spawned, coordinated through Agora, and folded back into
the turn's cognitive contract. Wire the already-implemented-but-dead
`CognitiveRoleWorkflow` into the production turn pipeline behind an
`IntentInterpreter` gate.

> Dependency: **B requires E′ (Per-Child Capability Attenuation)** —
> `docs/plans/2026-07-30-per-child-capability-attenuation-design.md`. The role
> children spawned here MUST be attenuated on spawn so that
> `child-effective ⊆ parent-effective`. E′ is Wave 0; B is Wave 1 and MUST NOT
> land before E′ closes the spawn choke point at
> `crates/executive/src/application/agent_control/mod.rs:856`.

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
- `run_reviewer_tester_fixer` (`:388-679`) drives `[Tester, Reviewer, Fixer]`
  with `StageDecision`-gated handoffs.
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
- The turn pipeline already **creates a Root cognitive task** per thread:
  `TurnPipeline::run` (`turn_pipeline.rs:337`, returns
  `anyhow::Result<serde_json::Value>`) calls `ensure_root_cognitive_task`, which
  guards on Agora presence (`turn_pipeline.rs:183`), lists tasks
  (`:191`), and commits a `CognitiveTaskNode{ role: Root, stage: Contract }`
  (`:196-228`).
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

1. Introduce a typed `IntentInterpreter` that maps a turn objective to a
   `TaskDecomposition` — either `Simple` (unchanged single-agent turn) or
   `RoleGraph` (drive `CognitiveRoleWorkflow`). It must be **host-authored /
   pre-turn**, never a prompt-phrase match inside the execution loop
   (`turn.rs:53-55`).
2. Add the **single production caller** that constructs
   `AgentControlRoleInvoker` + `CognitiveRoleWorkflow` and invokes
   `run_full_coding_workflow` from the turn pipeline, after the Root cognitive
   task exists (`turn_pipeline.rs:225`).
3. Fold the `FullCodingWorkflowReceipt` (`cognitive_role_workflow.rs:203-206`)
   back into the turn's `CognitiveTaskContract` obligations and terminal result,
   so the turn is only "done" when the graph's artifacts are committed.
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
  ├─ IntentInterpreter::interpret(objective, ctx) ─► TaskDecomposition   (NEW, cognit)
  │        │
  │        ├─ Simple ─────────────────────────► normal ReActLoop turn (unchanged)
  │        │                                          step.rs:99 / tool_exec.rs:23
  │        └─ RoleGraph{ scope, evidence } ────► drive workflow ▼
  │
  ▼
CognitiveRoleWorkflow::run_full_coding_workflow      cognitive_role_workflow.rs:363
  for role in [Planner, Explorer, Executor,
               Tester, Reviewer, Fixer]:
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

### 3.2 Decision D1 — where the IntentInterpreter lives

| Approach | Summary | Trade-off |
|---|---|---|
| **D1-A** In-loop LLM classification | ReActLoop asks the model mid-turn whether to decompose | **Rejected.** Violates `turn.rs:53-55` (requirements must not be derived "by matching prompt phrases in the execution loop"); couples planning to the streaming loop; hard to test deterministically. |
| **D1-B** Pure host metadata | Client sets a new `TurnRequirement::DecomposeRoleGraph{…}` consumed at `session.rs:336` | Cleanest w.r.t. the constraint, but pushes the *decision* onto the client; most turns carry no such metadata, so the loop never self-activates. |
| **D1-C** Pre-turn interpreter service (RECOMMENDED) | New `IntentInterpreter` in `cognit::core`, invoked **once** by `TurnPipeline` before the ReActLoop, producing a typed `TaskDecomposition` | One bounded classification per turn, outside the execution loop; host owns the call; deterministic to unit-test; may optionally use a single model call but the *result is typed*, satisfying the "host-authored, not in-loop phrase-match" rule. |

**Recommendation: D1-C**, with a D1-B escape hatch. Add
`crates/cognit/src/core/intent_interpreter.rs`:

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

pub trait IntentInterpreter: Send + Sync {
    fn interpret(&self, objective: &str, ctx: &InterpretCtx) -> TaskDecomposition;
}
```

`RoleGraph` fields map 1:1 onto `CodingWorkflowRequest`
(`cognitive_role_workflow.rs:161-170`), so no impedance mismatch. The default
production impl is a bounded, deterministic classifier (task-kind heuristic over
`CognitiveTaskKind`, `cognitive_task.rs:16-21`); a model-backed impl is a drop-in
that still returns the typed enum. A `TurnRequirement::DecomposeRoleGraph`
(new variant at `turn.rs:57`) short-circuits the interpreter to `RoleGraph`
(the D1-B escape hatch) when a client already knows.

Rationale: keeps the ReActLoop untouched, honors §1's "host-authored" invariant,
and reuses the existing `ensure_root_cognitive_task` seam (`turn_pipeline.rs:225`)
as the injection point — the interpreter runs immediately after the Root node is
committed.

### 3.3 Decision D2 — fixed vs configurable role graph

| Approach | Summary | Trade-off |
|---|---|---|
| **D2-A** Fixed canonical graph (RECOMMENDED) | Use the hard-coded `[Planner, Explorer, Executor]` (`:236-240`) + `[Tester, Reviewer, Fixer]` already in `CognitiveRoleWorkflow` | Zero new surface; matches the canonical `CognitiveRole` set and `CognitiveRoleProfile::canonical` (`cognitive_workflow.rs:103`); single authority. |
| **D2-B** Configurable DAG | Drive the order from `CognitiveTaskNode.dependencies` + `required_artifact_kinds` (`cognitive_workflow.rs:293-309`) | More flexible, but adds a scheduler, cycle checks, and per-turn config; premature for Wave 1. |

**Recommendation: D2-A.** The canonical graph is implemented and host-validated
(`Reviewer` independence is even asserted at
`cognitive_workflow.rs:854`). Model the DAG in the Agora task node now
(the fields already exist) so D2-B is a later, additive change with no rewrite.

### 3.4 Decision D3 — how Agora carries handoffs

| Approach | Summary | Trade-off |
|---|---|---|
| **D3-A** Typed artifacts + StageDecision (RECOMMENDED) | Each role terminal → `CognitiveArtifactEnvelope`; only host-validated artifacts advance; gates via `StageDecision` (`Accept/Reject/Replan/Repair/Block`, `cognitive_workflow.rs:525-531`) | Already implemented (`commit_artifact_at` / `record_stage_decision_at` in `cognitive_workspace.rs`); version-checked; auditable; matches the module's stated invariant ("only host-validated artifacts and gate decisions advance", `cognitive_role_workflow.rs:2-3`). |
| **D3-B** Generic blackboard KV | Roles read/write the Agora `Blackboard`/`Workspace` | Untyped, no gate semantics, bypasses validation; regresses the invariant. |
| **D3-C** `task_graph` transitions only | Advance via `TaskGraph::transition` (`task_graph/mod.rs:149`) | Coarse (`Pending/Running/Done/Failed`, `:9-14`); loses artifact provenance. |

**Recommendation: D3-A.** It is the existing mechanism and the only one that
preserves "prose cannot advance the stage" (`cognitive_role_workflow.rs:96`).

### 3.5 Decision D4 — sync vs async role execution

| Approach | Summary | Trade-off |
|---|---|---|
| **D4-A** Sequential (RECOMMENDED) | Spawn role → `wait` terminal → gate → next role, as `invoke` already does (`:99-133`) | Deterministic; the version-checked commits (`expected_workspace_version`, `CodingWorkflowRequest:164`) assume serialized advancement; simplest cancellation story. |
| **D4-B** Parallel independent roles | e.g. Explorer ∥ Tester via `JoinSet` + optimistic-version reconciliation | Latency win, but needs conflict handling on `VersionConflict` (`crates/agora/src/workspace/mod.rs`) and a merge policy; defer. |

**Recommendation: D4-A** for Wave 1. Bounded latency is acceptable; correctness
and E′ budget accounting are simpler when roles are serialized.

### 3.6 Types / functions to add or change

- **NEW** `crates/cognit/src/core/intent_interpreter.rs`: `IntentInterpreter`
  trait, `TaskDecomposition`, `InterpretCtx`, a deterministic default impl.
  Re-export at `crates/cognit/src/lib.rs` (next to `:32`).
- **CHANGE** `crates/cognit/src/core/planner.rs`: reconcile per §8 (see Open
  Decision O1). Recommended: mark `Planner`/`Reasoner` `#[deprecated]` and drop
  the `lib.rs:32` re-export, since the LLM Planner **role** — not this toy — is
  the planning authority. (No behavior in the loop depends on it today.)
- **NEW** production wiring in `crates/executive/src/application/turn_pipeline.rs`:
  a `drive_role_graph(&self, request, decomposition, owner)` method invoked from
  `run` (`:337`) right after `ensure_root_cognitive_task`, that builds
  `CodingWorkflowRequest` and calls
  `CognitiveRoleWorkflow::run_full_coding_workflow`.
- **CHANGE** `crates/executive/src/host/daemon/bootstrap/services.rs`: construct
  `AgentControlRoleInvoker::new(control, root, parent, …, profiles,
  wait_timeout_ms)` (`cognitive_role_workflow.rs:42-63`) with a
  `HashMap<CognitiveRole, RoleLaunchProfile>` from the canonical profiles, and
  hand `CognitiveRoleWorkflow` to `TurnPipeline` (alongside the existing
  `agora` field, `turn_pipeline.rs:47`).
- **CHANGE** `turn_pipeline.rs`: after the workflow returns
  `FullCodingWorkflowReceipt`, fold artifact ids / final version into the turn
  result and mark the corresponding `CognitiveTaskContract` obligations
  satisfied (see O4 for the exact obligation encoding).
- **REUSE UNCHANGED**: `AgentControlRoleInvoker::invoke`
  (`cognitive_role_workflow.rs:84`), `CognitiveWorkspaceCoordinator`
  (`cognitive_workspace.rs`), `AgoraRegistry::{project_task,list_tasks}`
  (`ops/mod.rs:365,374`), `AgentControlService::spawn` (`agent_control/mod.rs:856`
  — receives already-attenuated inputs from E′).

## 4. Error handling

- **Role runtime failure** — `invoke` already fails closed on a non-`Succeeded`
  terminal (`cognitive_role_workflow.rs:138-143`) or non-JSON output (`:147-151`).
  The workflow records a `StageDecision{ Reject | Replan }` and stops advancing.
  `drive_role_graph` maps a workflow `Err` to: (a) if any artifact committed,
  return a partial receipt + surface a non-fatal turn note; (b) otherwise fall
  back to a normal single-agent ReActLoop turn. Never panics the turn.
- **Planner-role failure / invalid Plan** — the artifact gate rejects at
  validation (`Plan` validated at `:835`); recorded as `Reject`; workflow aborts
  gracefully. The `Fixer` role covers `StageDecisionKind::Repair`
  (`:430`) for downstream fixable findings, not for a missing plan.
- **Agora unavailable** — `TurnPipeline` already guards `let Some(agora) = …`
  (`turn_pipeline.rs:183`). If Agora is `None`, the interpreter MUST NOT select
  `RoleGraph` (no substrate) → single-agent turn. Mid-workflow, a
  `VersionConflict` from `commit_artifact_at` surfaces as
  `CognitiveWorkspaceError`; policy: re-project the task (fresh
  `expected_workspace_version`) and retry the single commit once, else abort the
  stage with `Block`.
- **Budget / depth limits** — role spawns already set `max_depth: 1`
  (`cognitive_role_workflow.rs:119`), capping recursion; E′ additionally caps
  each budget dimension to the parent minimum. `drive_role_graph` enforces a
  workflow-level ceiling = Σ role budgets ≤ parent turn budget; the per-role
  wait is bounded by `wait_timeout_ms.min(budget.max_elapsed_ms)` (`:130`).
- **Cancellation** — the turn `CancellationToken` must abort in-flight role
  `wait`s. Sequential execution (D4-A) makes this a single await point per role;
  on cancel, `drive_role_graph` stops before the next spawn and records a
  `Cancelled` task status (`CognitiveTaskStatus::Cancelled`,
  `cognitive_workflow.rs:282-290`). (Note: `SubAgentExecutionContext`,
  `sub_agent.rs:65-70`, carries no cancel handle; the workflow owns cancellation
  at the `wait` boundary, not inside the child.)

## 5. Verification

**Unit tests** — `crates/cognit/src/core/intent_interpreter.rs`:

- `interpret` returns `Simple` for a plain objective; `RoleGraph` for a
  code-change objective; deterministic, no network.
- `RoleGraph` field population maps cleanly onto `CodingWorkflowRequest`.
- A `TurnRequirement::DecomposeRoleGraph` short-circuits to `RoleGraph` (D1-B).

**Integration tests** — `crates/executive` (extend the existing
`cognitive_role_workflow.rs` test harness at `:1031+` with a wired invoker):

- `role_graph_activation`: a turn whose objective triggers `RoleGraph` drives
  Planner → Explorer → Executor → Tester → Reviewer → Fixer; assert one
  `CognitiveArtifactEnvelope` per stage is committed and each `StageDecision` is
  `Accept`; assert `FullCodingWorkflowReceipt` artifact ids are non-empty and the
  turn result references them.
- `agora_none_falls_back`: with `agora: None` (`turn_pipeline.rs:183`), no role
  graph runs; a normal turn returns.
- `role_failure_is_graceful`: a role runtime returning non-`Succeeded` yields a
  `Reject`/`Replan` decision and a partial receipt, not a turn panic.
- `attenuation_prerequisite` (cross-refs E′): a role child requesting a superset
  of the parent's tools is spawned with the intersection; asserts a
  `CapabilityAttenuated` event (per the E′ doc §3.4). This test is the seam where
  B and E′ meet.
- `existing cognitive_role_workflow tests stay green` (`:1300, :1412, :1461`).

**Commands** (via the wrapper, narrowest first):

```
bash scripts/cargo-agent.sh test -p cognit
bash scripts/cargo-agent.sh test -p executive
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

## 6. Files touched

- `crates/cognit/src/core/intent_interpreter.rs` — **new**: `IntentInterpreter`,
  `TaskDecomposition`, default impl + unit tests.
- `crates/cognit/src/lib.rs` — re-export the interpreter; drop/deprecate the
  `Planner` re-export at `:32` (O1).
- `crates/cognit/src/core/planner.rs`, `reasoner.rs` — deprecate per §8 (O1).
- `crates/fabric/src/include/turn.rs` — add
  `TurnRequirement::DecomposeRoleGraph{…}` variant (`:57`) for the D1-B hatch.
- `crates/executive/src/application/turn_pipeline.rs` — add `drive_role_graph`,
  call it from `run` (`:337`) after `ensure_root_cognitive_task`; fold the
  receipt into the turn result; hold a `CognitiveRoleWorkflow` handle.
- `crates/executive/src/host/daemon/bootstrap/services.rs` — construct
  `AgentControlRoleInvoker` + `CognitiveRoleWorkflow` from canonical role
  profiles and inject into `TurnPipeline` (near `:86-102`).
- `crates/executive/src/application/cognitive_role_workflow.rs` — no logic
  change expected beyond a public constructor/accessor if bootstrap needs it;
  the `run_*` methods are reused as-is.
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

## Open decisions for review (codex)

- **O1 — Fate of the toy `cognit::Planner`.** Recommended: deprecate/remove
  (`planner.rs:17`, `reasoner.rs:25`, re-export `lib.rs:32`) rather than wire a
  second planning authority. Reviewer to confirm nothing external depends on the
  re-export and that §8 single-authority is best served by deletion vs.
  repurposing it as an in-child helper the Planner *role* calls.
- **O2 — May the default `IntentInterpreter` make one pre-turn model call?**
  D1-C returns a typed result either way, but a model-backed classifier is still
  a per-turn inference cost. Reviewer to set the policy (deterministic-only vs.
  model-allowed-pre-turn) against `turn.rs:53-55`.
- **O3 — One invoker convention.** B's `AgentControlRoleInvoker` and the eval
  kernel's `AgentControlBenchmarkRuntime` (`capability_benchmark.rs:374`) spawn
  near-identical `AgentSpawnRequest`s. Reviewer to decide whether to unify them
  behind one helper before B lands.
- **O4 — Receipt → obligation encoding.** How should
  `FullCodingWorkflowReceipt` satisfy the turn contract: a new
  `RequiredAction::RunRoleGraph{…}` variant
  (`cognitive_task.rs:27`), or via `deliverables` + `validation_requirements`
  already carried by `CognitiveTaskContract` (`cognitive_task.rs:7-13`)?
- **O5 — Cancellation depth.** B cancels at the `wait` boundary
  (`cognitive_role_workflow.rs:127`); it does not interrupt a child mid-flight
  (no cancel handle in `SubAgentExecutionContext`, `sub_agent.rs:65-70`).
  Reviewer to confirm that boundary-level cancellation is sufficient for Wave 1.
