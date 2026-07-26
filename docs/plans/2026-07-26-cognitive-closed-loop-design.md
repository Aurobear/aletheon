# Cognitive Closed Loop Design

## Status

Approved direction, pending written-spec review before implementation planning.

## Objective

Make `cognit` the authoritative task-cognition engine for Aletheon. A turn must
understand the user's required method and deliverables, maintain explicit
knowledge and evidence state, verify important claims, and prove completion
before the runtime accepts a final answer.

The broader program goal is practical work capability, not completion policing
alone. Cognit must be able to investigate a repository by following production
logic, plan and perform edits, run the narrowest authoritative validation,
recover from failures, and use native or external agents without losing the
project context and working discipline that make those agents effective.

This phase deliberately precedes deeper Dasein enforcement and self-evolution.
Dasein will initially consume grounded cognitive outcomes; it will not replace
task interpretation, evidence validation, or completion decisions.

## Problem

The current linear harness completes normally when the model returns no tool
calls. Its periodic reflection primarily observes tool counts, errors, timeout
streaks, and budget exhaustion. That protects loop health but does not prove
that the requested method ran, required evidence was collected, or final claims
are supported.

The failure observed in session `91cefb13-5386-45ba-9406-109bbacdf76c`
demonstrates the gap: the user requested analysis using Pi, but the native
linear harness only issued file-discovery/read calls and still accepted a final
answer. The implementation must prevent this class of false completion without
keying behavior to that prompt, language, repository, or fixed path.

## Design principles

1. **Runtime facts are authoritative.** Model prose cannot establish that an
   agent ran, a command succeeded, a test passed, or an async operation ended.
2. **Completion is typed state.** No-tool output is only a completion proposal.
3. **Evidence has levels.** Existence, implementation, production wiring,
   deterministic test evidence, and installed-runtime evidence are distinct.
4. **Deterministic checks come first.** Use an LLM verifier only for semantic
   coverage and inference quality that typed checks cannot decide.
5. **Keep the ReAct loop small.** Task cognition is composed from focused
   components rather than added as more branches in `step.rs`/`tool_exec.rs`.
6. **Dasein receives experiences, not self-authored facts.** Cognitive outcomes
   may later update identity, capability belief, care, negativity, continuity,
   and narrative through explicit transition events.
7. **Evidence gates support work rather than replace it.** A task that honestly
   reports "not done" is safer than a false completion, but it is not capable;
   planning, execution, recovery, and validation must advance the task.
8. **Adopt proven mechanisms, not product-shaped copies.** Pi, Codex, and Grok
   Build inform distinct parts of the design while Aletheon retains unified
   runtime truth, persistence, security, Dasein, and conscious arbitration.
9. **Agent state is larger than model context.** Persistent task, evidence,
   Agora, memory, Dasein, and runtime state remain host-owned; each inference
   receives only a bounded, purpose-built projection.
10. **Multi-agent work communicates through typed artifacts.** Planner,
    executor, reviewer, and tester roles exchange contracts, plans, diffs,
    evidence, findings, and receipts through Agora rather than forwarding entire
    private transcripts.

## Reference-agent absorption map

The implementation must preserve source provenance in design and tests without
copying external product internals wholesale.

| Reference | Mechanism to absorb | Aletheon destination |
|---|---|---|
| Pi | Small, predictable model/tool/result loop; project instructions, skills, and focused coding tools provide most practical leverage | Keep the linear harness small; inject Aletheon-owned project context and skills into external-agent task packets |
| Codex | Explicit work discipline: inspect, plan when needed, edit, verify narrowly, continue until the request is resolved, and report residual gaps | Cognitive workflow policy, typed plan state, validation strategy, and final reporting contract |
| Grok Build | Completion requirements, todo/progress gates, terminal-task observation, subagent specialization, and claim-versus-tool-evidence audit | Required actions, progress auditor, completion gate, external-agent receipts, and evidence audit |

Pi demonstrates that a capable agent does not require a large cognitive loop;
the quality of model-visible context, tools, and instructions matters. Codex
demonstrates a strong inspect-edit-verify operating discipline. Grok Build
demonstrates that important completion claims can be checked against runtime
evidence and outstanding work. Aletheon combines these with persistent typed
state and later feeds grounded experience into Dasein.

### Pi agent-loop mechanics to absorb

The Pi reference is more than a general recommendation to keep the loop small.
Its concrete loop mechanics form the baseline behavior that Cognit must retain
or improve:

1. **Nested continuation loops.** An inner loop continues while tools or steering
   messages remain; an outer loop checks queued follow-up messages before the
   agent becomes idle.
2. **Steering at safe boundaries.** User input submitted during work is injected
   before the next model response without corrupting the assistant/tool-result
   ordering.
3. **Turn-boundary adaptation.** `prepareNextTurn` may replace context, model, or
   reasoning configuration after observing the assistant response and complete
   tool-result batch.
4. **Explicit stop hook.** `shouldStopAfterTurn` allows the host to terminate a
   turn from authoritative state rather than relying only on model output.
5. **Follow-up draining.** A model that would otherwise stop must first consume
   queued follow-up work.
6. **Context transformation at the model boundary.** Internal Agent messages are
   transformed and converted only immediately before inference, keeping runtime
   state richer than provider message schemas.
7. **Complete event lifecycle.** Agent, turn, message, tool-start, tool-update,
   tool-end, and agent-end events expose what actually happened.
8. **Truncated-call safety.** If model output ends because of its token limit,
   potentially truncated tool arguments are rejected and returned as errors for
   re-issuance rather than executed.
9. **Declared execution ordering.** Tool batches may execute in parallel, while
   tools marked sequential force ordered execution.
10. **Error and abort terminality.** Provider error or abort ends the loop with
    that status; it is never reinterpreted as normal completion.

Aletheon should not embed these as Pi-specific branches. They become generic
`CognitiveLoopDriver` semantics used by native Cognit turns and represented at
the external-agent boundary:

```rust
pub trait CognitiveLoopPolicy: Send + Sync {
    fn prepare_next_turn(
        &self,
        snapshot: &CompletedTurnSnapshot,
        state: &CognitiveTurnState,
    ) -> NextTurnDecision;

    fn evaluate_stop(
        &self,
        proposal: &CompletionProposal,
        state: &CognitiveTurnState,
    ) -> CompletionDecision;
}
```

The proposed Cognit completion gate supplies `evaluate_stop`; steering,
follow-up draining, context transformation, and event emission remain runtime
responsibilities. This separation keeps Pi's useful loop discipline while
allowing Aletheon's evidence, persistence, security, Dasein, and conscious-field
layers to operate on typed state.

## Practical work capability model

Completion is meaningful only if Cognit can make progress across four work
phases.

### Investigate

- Batch-read known entry files before scoped discovery.
- Build a repository map from file contents rather than extension inventories.
- Trace selected behavior vertically from entry point through production wiring
  to tool/runtime effects.
- Record open questions and choose the next read by expected information gain.
- Distinguish path discovery from content evidence and absence verification.

### Plan

- Use a typed plan for multi-step analysis or implementation.
- Associate each step with a requirement, expected evidence, and validation.
- Re-plan after contradictory evidence, repeated errors, or a changed user
  instruction; do not merely inject a generic "try another approach" message.
- Keep simple tasks direct so planning does not become ceremony.

### Execute

- Select native execution, Pi, or a specialized child from typed capability and
  explicit user method requirements.
- Provide the selected worker with the task contract, project instructions,
  allowed skills/tools, workspace scope, and acceptance criteria.
- Support the coding loop: inspect affected code, edit narrowly, inspect the
  resulting diff, and preserve unrelated user changes.
- Treat tool admission, execution, output, and authoritative terminal receipt as
  separate states.

### Verify and recover

- Choose the narrowest deterministic test, check, lint, or runtime probe that
  validates the change or conclusion.
- Attribute failures to provider, tool, environment, implementation, test, or
  task-model causes before deciding the next action.
- Retry only transient failures; change strategy for deterministic failures.
- Reconcile final prose with diffs, test output, session evidence, audit records,
  daemon logs, and installed-runtime facts where applicable.
- Report success, partial completion, blocked state, and residual risk as
  distinct outcomes.

## Agent state, model context, and information flow

An Aletheon Agent is not identical to one LLM request and its context window.
The Agent is the persistent host-side process formed from task state, workspace,
memory, identity, policy, runtime receipts, and an inference loop. The model is
a replaceable reasoning component that receives a projection of that state.

```text
Persistent Agent state
  - CognitiveTaskContract / plan / obligations
  - Agora workspace and task graph
  - evidence and terminal receipts
  - canonical session history
  - Mnemosyne recall
  - Dasein self projection
  - policy, budgets, capabilities
                 |
                 v
          ContextProjector
                 |
        bounded role-specific view
                 |
                 v
            LLM inference
                 |
       text / tool calls / proposals
                 |
                 v
 authoritative execution + Agora commit
                 |
                 +----> next projection
```

The runtime must never serialize all persistent Agent state into every model
request. Instead, `ContextProjector` assembles these layers under separate
budgets:

1. Stable system and project instructions.
2. Current user request and task contract.
3. Current role, workflow stage, assigned task node, and acceptance criteria.
4. The newest complete conversation/tool units required for local coherence.
5. Selected Agora items relevant to the task node.
6. Bounded recalled memory with provenance and untrusted-data marking.
7. Bounded Dasein self projection.
8. Explicit references to larger durable artifacts that tools may fetch on
   demand instead of embedding their full contents.

Every projected fragment records provenance, source version, selection reason,
token/byte cost, and whether it is instruction-bearing or untrusted data. The
projection receipt is persisted so a later reviewer can reconstruct what the
model actually saw.

Large code, logs, diffs, and child transcripts stay in durable storage. The
model receives summaries plus artifact references and retrieves exact content
when it is relevant. Compaction may replace old conversational text, but it may
not remove task obligations, acceptance state, evidence identity, or unresolved
review findings.

## Agora as the shared cognitive workspace

Agora is part of the core design, not an optional context decoration. It is the
shared workspace through which cognitive roles coordinate. The initial scope is
one `AgoraSpaceId` per root task/session, with explicit bridges for approved
cross-session knowledge. A single unrestricted machine-global mutable space
would mix users, tasks, permissions, and stale hypotheses, so global awareness
is built from scoped spaces and selected broadcasts rather than one shared dump.

Agora owns the live collaborative projection:

```text
Workspace
  |- TaskGraph       role tasks, dependencies, stage state
  |- Blackboard      accepted facts, requirements, decisions
  |- Scratchpad      provisional notes and hypotheses
  |- Evidence        artifact and runtime-evidence references
  |- CandidatePool   competing plans, claims, and next actions
  |- Attention       selection priorities
  |- Broadcast       bounded globally available selection for the space
  `- Trace           proposals, commits, rejections, and attribution
```

Mnemosyne remains the long-term retention system. Agora contains mutable working
state and durable commit/broadcast history for replay; it does not replace
long-term memory. Dasein consumes selected grounded workspace outcomes to update
the lived-self projection. Cognit reads and proposes workspace changes but may
only treat committed Agora content as shared fact.

### Agora artifact contract

Agents communicate by committing typed artifacts:

```rust
pub enum CognitiveArtifact {
    TaskContract(CognitiveTaskContract),
    Plan(CognitivePlan),
    Investigation(InvestigationReport),
    ChangeSet(ChangeSetReceipt),
    Validation(ValidationRecord),
    Review(ReviewFindingSet),
    Evidence(EvidenceRecord),
    Decision(StageDecision),
    AgentResult(AgentResultReceipt),
}
```

An artifact contains author/process identity, task-node ID, source versions,
evidence references, confidence, lifecycle status, and content digest. A child
Agent's prose is not automatically a shared fact; its result is a proposal until
the owning stage validates and commits it.

## Multi-agent cognitive workflow

Complex work uses a workflow graph rather than asking one model context to plan,
implement, review, and certify itself. Roles are logical capabilities and may
run on Pi, a native runtime, or another configured provider.

```text
                 +----------------+
User requirement | Intent/Contract|
                 +-------+--------+
                         |
                         v
                 +-------+--------+
                 | Planner        |
                 | plan + criteria|
                 +-------+--------+
                         |
                Plan acceptance gate
                         |
              +----------+----------+
              |                     |
              v                     v
       +------+-------+      +------+-------+
       | Explorer     |      | Executor     |
       | evidence map |      | diff/receipt |
       +------+-------+      +------+-------+
              |                     |
              +----------+----------+
                         |
                 Execution gate
                         |
              +----------+----------+
              |                     |
              v                     v
       +------+-------+      +------+-------+
       | Reviewer     |      | Tester       |
       | findings     |      | validation   |
       +------+-------+      +------+-------+
              |                     |
              +----------+----------+
                         |
                  Acceptance gate
                    |           |
                  reject      accept
                    |           |
                  Fixer      Synthesizer
                    |           |
                    +-----------+
```

### Role responsibilities

- **Planner:** decomposes requirements into Agora task nodes, dependencies,
  expected artifacts, and acceptance criteria. It does not certify its own plan.
- **Explorer:** performs read-only investigation and commits evidence-linked
  findings. It cannot mutate the workspace.
- **Executor:** owns assigned files/worktree scope and produces a change-set
  receipt and implementation evidence.
- **Reviewer:** checks requirements, design boundaries, diff correctness,
  unsupported claims, and regression risks. It emits actionable findings.
- **Tester:** runs authoritative focused validation, separates implementation
  failures from environment/provider failures, and emits terminal evidence.
- **Fixer:** addresses named reviewer/tester failures only and returns a new
  change-set receipt for re-review.
- **Synthesizer:** produces the user-facing result from accepted artifacts; it
  cannot promote rejected or unverified artifacts into success claims.

### Stage gates

Each edge is an acceptance boundary with deterministic minimum conditions:

| Gate | Required inputs | Accept condition |
|---|---|---|
| Plan | Task contract, plan artifact | Every explicit requirement maps to a task node and acceptance criterion |
| Execution | Accepted plan, investigation/change artifacts | Assigned nodes have terminal receipts and no missing dependencies |
| Review | Change set, evidence, requirement map | No blocking finding; claims do not exceed evidence |
| Test | Validation plan, authoritative output | Required commands reached terminal success; skipped checks are explicit |
| Final | Accepted review and test decisions | All required nodes done, no blocking finding, completion audit passes |

Gate rejection creates typed feedback on the affected task node and schedules a
specific re-plan or fixer edge. It does not paste a vague review paragraph into
every Agent context.

Simple tasks bypass unnecessary roles. The workflow policy may run one native
Agent through inspect/edit/verify when risk and scope are small, but the same
artifact and gate contracts still apply.

## Scope

### Included in the first implementation increment

- Typed task contract for explicit required actions and deliverables.
- Per-turn cognitive state containing requirements, observations, validations,
  and unresolved obligations.
- Evidence records derived from authoritative tool and runtime results.
- Deterministic completion gate applied when the model proposes to stop.
- Recovery feedback that tells the model exactly which obligations remain.
- Final claim audit for mechanically verifiable execution claims.
- Session/protocol persistence sufficient to inspect why completion was
  accepted, retried, or rejected.
- Focused unit and integration tests for required-agent execution, terminal
  evidence, unsupported claims, and ordinary turns without special methods.
- Shadow-mode Dasein outcome publication; no new Dasein enforcement.
- A typed inspect/plan/execute/verify workflow state, initially exercised by
  repository analysis and required-agent tasks.
- External-agent task packets carrying Aletheon-owned project instructions,
  workspace scope, evidence requirements, and acceptance criteria even when Pi
  session/context/skill discovery remains disabled for isolation.
- Validation selection and failure classification sufficient to drive a
  concrete next action rather than only record an error count.
- Agora-backed task nodes and cognitive artifacts for planner, executor,
  reviewer, and tester handoffs.
- Role-specific context projection receipts proving what each model invocation
  received and why it was selected.
- Deterministic stage gates for plan, execution, review, test, and final
  acceptance, with simple-task bypass policy.

### Explicitly deferred

- General natural-language theorem proving or universal factual verification.
- Broad redesign of Agora, Mnemosyne, Dasein, or the Conscious Workspace.
- Automatic mutation of prompts, policies, completion criteria, or boundaries.
- Production enforcement by Conscious Workspace; it remains in `Observe` while
  cognitive decision quality is measured.
- Benchmarking Pi, Codex, and Grok Build. The benchmark follows once the same
  completion/evidence contract can instrument every run.
- Broad autonomous coding optimization. The first increment establishes the
  shared workflow and proves it on bounded analysis and edit/validate scenarios;
  later increments tune tool ergonomics and model policies from benchmark data.
- Machine-global mutable Agora state shared indiscriminately across sessions.
  Cross-session awareness requires explicit projection, provenance, policy, and
  retention boundaries.

## Architecture

```text
User input
   |
   v
IntentInterpreter ----> CognitiveTaskContract
                              |
                              v
                       CognitiveTurnState
                              |
                              v
                   ReActLoop / agent runtime
                              |
                    tool/runtime receipts
                              |
                              v
                        EvidenceLedger
                              |
                              v
                    CognitiveProgressAudit
                              |
                    +---------+---------+
                    |                   |
                 Continue          CompletionGate
                    |                   |
                    +<-- recovery ------+
                                        |
                                     Accept
                                        |
                              GroundedOutcomeEvent
                                        |
                              Dasein shadow transition
```

## Components

### 1. Cognitive task contract

The contract represents what must be true for the turn to complete.

```rust
pub struct CognitiveTaskContract {
    pub objective: String,
    pub task_kind: CognitiveTaskKind,
    pub required_actions: Vec<RequiredAction>,
    pub deliverables: Vec<Deliverable>,
    pub validation_requirements: Vec<ValidationRequirement>,
}

pub enum RequiredAction {
    InvokeAgent { runtime: AgentRuntimeKind },
    InvokeTool { tool_name: String },
    ObserveTerminal { operation_id: OperationId },
}
```

Only explicit user requirements and typed workflow policy may create required
actions. The interpreter must not turn preferences or incidental wording into
new obligations.

The first interpreter is intentionally conservative. It resolves explicit
agent-runtime requests using the installed runtime registry and otherwise
returns no required action. Ambiguous requests remain unresolved rather than
being guessed.

### 2. Cognitive turn state

The state is owned at the turn boundary and survives individual inference
iterations.

```rust
pub struct CognitiveTurnState {
    pub contract: CognitiveTaskContract,
    pub phase: CognitiveWorkPhase,
    pub plan: Option<CognitivePlan>,
    pub observations: Vec<Observation>,
    pub validations: Vec<ValidationRecord>,
    pub outstanding: Vec<Obligation>,
    pub completion_attempts: u32,
}
```

`CognitiveWorkPhase` is one of `Investigate`, `Plan`, `Execute`, `Verify`, or
`Synthesize`. Phase transitions are recorded so the runtime can distinguish
productive work, validation, and premature final narration.

It is separate from model message history. Compaction may summarize messages,
but it must not erase contract, receipt, validation, or outstanding-obligation
state.

### 3. Evidence ledger

Evidence is created by adapters from authoritative outputs, not by parsing the
model's reasoning.

```rust
pub enum EvidenceLevel {
    Located,
    Observed,
    Implemented,
    ProductionWired,
    DeterministicallyVerified,
    RuntimeVerified,
}

pub struct EvidenceRecord {
    pub source: EvidenceSource,
    pub level: EvidenceLevel,
    pub terminal_status: TerminalStatus,
    pub locator: EvidenceLocator,
    pub digest: Option<String>,
}
```

Tool submission is not terminal evidence. Async operations satisfy obligations
only after an authoritative terminal snapshot, event, or durable receipt.

### 4. Progress auditor

The current reflection engine remains responsible for loop-health signals. A
new progress auditor compares the task contract with cognitive state.

```rust
pub enum ProgressDecision {
    Continue { missing: Vec<Obligation> },
    NeedsValidation { requirements: Vec<ValidationRequirement> },
    WaitingForTerminalEvidence { operations: Vec<OperationId> },
    Complete,
    Blocked { reason: String },
}
```

The auditor is deterministic for the first increment. It does not decide
whether an arbitrary architectural opinion is correct; it decides whether
declared required actions and terminal validations exist.

It also detects lack of progress from typed state: repeatedly reading without
resolving an open question, repeatedly issuing the same failed operation, or
entering synthesis while required plan steps remain. Its recovery response must
name a concrete missing obligation or next validation rather than emit a generic
reflection summary.

### 5. Completion gate

When the model returns no tool calls, the harness submits a completion proposal
to the gate. The gate either accepts it or injects a structured recovery item:

```text
[cognitive_completion_rejected]
- required agent runtime `pi` was not invoked
- terminal evidence for operation `<id>` is missing
- validation `<name>` has not completed
Continue the task. Do not claim completion until these obligations are met.
```

Retries are bounded by policy. Exhaustion produces an explicit incomplete or
blocked outcome; it must not be converted to a successful assistant message.

### 6. Claim-evidence audit

The first claim audit is deliberately narrow and deterministic. It recognizes
claims represented by typed final-answer annotations or generated from the
contract, including:

- a required external agent was used;
- an operation completed;
- a validation or test passed;
- a required deliverable was produced.

It does not attempt unrestricted natural-language fact checking. A later
semantic verifier may inspect broader conclusions, but it cannot override a
deterministic contradiction.

### 7. Dasein integration

After the completion decision, Cognit publishes a grounded outcome:

```rust
pub enum CognitiveExperience {
    RequirementSatisfied { requirement_id: String, evidence: Vec<EvidenceId> },
    CompletionRejected { missing: Vec<String> },
    ValidationFailed { name: String, evidence: EvidenceId },
    FalseCompletionPrevented { unsupported: Vec<String> },
    TaskCompleted { evidence: Vec<EvidenceId> },
}
```

Initially these events are observable inputs to Dasein. They may update
negativity, continuity, capability belief, and narrative projections, but they
do not give Dasein authority to change completion results. That authority can
be considered only after shadow-mode evidence shows better decisions without
new false positives.

### 8. External-agent execution packets

Aletheon may continue to disable Pi-owned session persistence, implicit context
discovery, unreviewed extensions, and approval handling. The replacement cannot
be an empty generic prompt. `executive` constructs an explicit task packet:

```rust
pub struct AgentTaskPacket {
    pub contract: CognitiveTaskContract,
    pub project_instructions: Vec<ProjectInstruction>,
    pub workspace_roots: Vec<WorkspaceRoot>,
    pub allowed_capabilities: Vec<CapabilityId>,
    pub expected_evidence: Vec<ValidationRequirement>,
    pub acceptance_criteria: Vec<CompletionCriterion>,
}
```

The packet is versioned and persisted with the child invocation receipt. This
allows Pi's simple loop, a native child, or a future adapter to receive the same
work definition without duplicating Aletheon's control plane.

### 9. Repository-analysis workflow

The first specialized cognitive workflow follows:

```text
scope -> entry-file batch read -> focused discovery -> production call-chain
trace -> maturity classification -> deterministic/runtime verification ->
counter-evidence search -> synthesis
```

Capability maturity is recorded as `Located`, `Implemented`, `ProductionWired`,
`DeterministicallyVerified`, or `RuntimeVerified`. Comparative claims require
evidence collected under the same stated evaluation dimensions; otherwise the
comparison is explicitly incomplete.

### 10. Coding work loop

The first bounded coding workflow follows:

```text
requirement -> inspect affected symbols -> failing/characterizing check ->
narrow edit -> diff inspection -> focused validation -> failure attribution ->
repair or completion audit
```

Repository instructions determine the authoritative commands. The workflow
must retain unrelated worktree changes and must not report diagnostic binaries
or isolated daemons as deployment acceptance.

### 11. Workflow and context projection

`executive` binds role runtimes to Agora task nodes and invokes Cognit with a
role-specific projection. Cognit produces proposals and tool actions;
`executive` records authoritative receipts and applies gate decisions; Agora
stores the shared working artifacts. No role receives another role's entire
message history by default.

The planner-to-worker handoff contains the accepted plan node, relevant project
instructions, selected evidence, workspace scope, and acceptance criteria. The
worker-to-reviewer handoff contains the requirement map, change receipt, diff
artifact, validation evidence, and unresolved risks. Tester output is consumed
as a typed validation artifact rather than summarized success prose.

## Data ownership and boundaries

- `cognit` owns task cognition, progress evaluation, and completion proposals.
- `fabric` owns only stable cross-crate protocol types required by ports.
- `executive` binds agent/runtime receipts, session persistence, and Dasein
  publication to the Cognit ports.
- `corpus` continues to own tool execution and returns authoritative results.
- `dasein` consumes grounded cognitive experiences and maintains the lived-self
  projection; it does not infer execution success from prose.
- `interact` renders completion rejection and evidence status without inventing
  new semantics.

## Failure semantics

| Condition | Required outcome |
|---|---|
| Required agent never invoked | Completion rejected |
| Agent/tool started but no terminal receipt | Waiting or incomplete |
| Terminal provider error | Failed; never successful completion |
| Validation exits non-zero | Failed validation evidence |
| Completion retries exhausted | Explicit incomplete/blocked outcome |
| Evidence persistence fails | Fail closed for required evidence |
| Optional semantic verifier unavailable | Report verifier unavailable; deterministic gate remains authoritative |
| Dasein projection unavailable | Continue with grounded Cognit result and record degraded self-plane state |

## Testing strategy

### Deterministic tests

1. Explicit required Pi action with no invocation rejects completion.
2. Pi invocation without terminal receipt does not satisfy the requirement.
3. Successful terminal Pi receipt satisfies the required action.
4. Provider failure remains a failed outcome even when prose claims success.
5. Ordinary conversational turns with no required action retain normal behavior.
6. Compaction preserves contract and outstanding obligations.
7. Completion retry exhaustion cannot produce `completed_normally = true`.
8. Grounded outcomes published to Dasein match the completion decision.
9. Repository analysis cannot classify a capability as production-wired from a
   path listing or type declaration alone.
10. A multi-step cognitive plan cannot complete with a required step pending.
11. An external-agent packet contains project instructions, acceptance criteria,
   and evidence requirements while remaining within the configured workspace.
12. A deterministic validation failure produces a repair/re-plan decision, not
   a transient provider retry.
13. A bounded edit workflow records diff evidence and focused validation before
   reporting the change complete.
14. Steering submitted during tool execution is injected before the next model
    response and after the complete tool-result batch.
15. Queued follow-up input prevents the loop from becoming idle prematurely.
16. A token-truncated tool-call batch is not executed and can be safely reissued.
17. Sequential tools preserve provider order while an independent batch may run
    in parallel.
18. Provider error and user abort remain terminal non-success outcomes.
19. Each role receives only its selected Agora/task projection and a persisted
    receipt identifies every included fragment.
20. Planner output cannot advance to execution until the plan gate accepts all
    explicit requirement mappings.
21. Executor output cannot advance to final synthesis without independent
    review and required validation for workflows whose policy requires them.
22. Reviewer rejection schedules a bounded fixer/re-review edge with the same
    finding IDs; it does not lose or duplicate findings during compaction.
23. Agora version conflicts reject stale Agent proposals and cause refresh or
    re-plan rather than last-writer-wins corruption.
24. Data from another session is absent unless an explicit authorized
    cross-space projection selected it with provenance.

### Installed-runtime acceptance

Because the change affects agent/runtime/session behavior, final acceptance must
use `sudo bash scripts/aletheon.sh deploy`, prove release/installed/running
binary digest equality, observe stable systemd restart counters, and complete a
real request through `/usr/bin/aletheon` and the official user socket.

The real acceptance scenario must explicitly require Pi, verify a Pi terminal
receipt in persisted evidence, and compare the rendered answer with session,
audit, and daemon records. Any provider error or evidence disagreement fails
acceptance.

## Delivery slices

1. **Contract, work-phase, plan, and evidence types** — no behavior change.
2. **Agora cognitive artifacts and role-specific context projection** — task
   graph handoffs, projection receipts, and version-conflict behavior.
3. **Pi-derived loop-driver semantics** — steering, follow-up draining,
   turn-boundary adaptation, truncated-call rejection, execution ordering, and
   complete lifecycle events behind generic Cognit/runtime ports.
4. **Runtime receipt adapters, agent task packets, and persistence** —
   observation only.
5. **Planner/explorer/executor workflow and plan/execution gates** — prove typed
   multi-agent handoff on a bounded repository task.
6. **Reviewer/tester/fixer workflow and acceptance gates** — prove rejection,
   repair, re-review, and terminal validation.
7. **Repository-analysis workflow and maturity evidence** — prove grounded
   investigation without changing completion behavior.
8. **Completion gate in shadow mode** — record would-reject decisions.
9. **Enforce required-action, plan-step, and terminal-evidence checks** — bounded
   recovery with concrete missing obligations.
10. **Bounded coding inspect/edit/verify workflow** — focused validation and
   failure attribution.
11. **Claim audit and grounded Dasein events** — deterministic claims only.
12. **Comparative capability benchmark harness** — run identical bounded tasks
   through native Cognit and configured external agents; do not encode product
   names into production policy.
13. **Installed-runtime acceptance and regression documentation.**

Each slice must have focused tests and a separate reviewable commit. No slice
may introduce prompt-, language-, repository-, or checkout-specific production
behavior.

## Success criteria

The first implementation increment is complete only when:

1. An explicit required-agent request becomes a typed obligation.
2. The obligation can only be satisfied by an authoritative terminal receipt.
3. No-tool model output cannot bypass the completion gate.
4. Missing evidence causes bounded recovery or an explicit incomplete result.
5. Persisted session/audit records explain every completion decision.
6. Dasein receives a grounded outcome consistent with runtime truth.
7. Existing turns without required actions retain their established behavior.
8. Installed-runtime acceptance passes under repository policy.
9. Repository analysis demonstrates vertical production-path tracing and does
   not promote discovery evidence into runtime claims.
10. A bounded code change follows inspect, edit, diff review, and focused
    validation before completion.
11. Native and external agents receive an equivalent typed task definition and
    their inference rounds, provider retries, tool calls, and validation results
    remain separately observable.
12. The native loop safely handles steering, queued follow-ups, truncated tool
    calls, sequential/parallel execution, and provider error/abort outcomes.
13. Planner, executor, reviewer, tester, and fixer exchange typed Agora
    artifacts through enforced stage gates rather than full transcript copying.
14. Model requests are bounded role-specific projections; persistent Agent state
    remains outside the model context and is reconstructable from receipts.
