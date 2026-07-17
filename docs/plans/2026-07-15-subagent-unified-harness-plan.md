# SubAgentRuntime Unified Harness and Multi-Agent Implementation Plan

> **Status:** In progress — G08 fairness and strict Pi protocol integration remain open
> **Target branch:** `dev`
> **Baseline:** Aletheon `65f74981`
> **Reference:** Codex `1bbdb327`, completed Aletheon M3 runtime work
> **Execution rule:** Preserve `NativeCognitEngine v0`. External Pi/Codex-like runtimes remain supervised implementations below Aletheon-defined contracts.

## Code-Reality Update (2026-07-17)

> **Note:** The 10-row gap table in Section 2.2 was written against a prior code
> baseline (`65f74981`). As of 2026-07-17 most of those gap claims are stale. The
> implementation phases (A1-A10) reference components that already exist in code.
> The 141 task checkboxes remain unchecked in the original plan text.

### Per-gap assessment

| # | Original gap claim | Current status | Evidence |
|---|---|---|---|
| 1 | Duplicate Harness in `init.rs` | **STALE.** The duplicate 20-step LLM loop concern is addressed. | `init.rs` is now a 45-line request-handler accessor file. The real loop lives at `exec_session.rs:55`. The duplicate-loop risk no longer applies. |
| 2 | Blocking delegation (`AgentTool::execute` waits inline) | **STALE but nuanced.** `AgentTool` (`agent_tool.rs:125`) still does synchronous spawn+wait. However, production agent control uses `AgentControlService` with full async spawn/wait/send/cancel/list/inspect at `crates/executive/src/service/agent_control/mod.rs:572`. The old `AgentTool` is a compatibility wrapper. |
| 3 | Cancellation stub | **STALE.** `native_cognit.rs` has real `CancellationToken` checks at lines 232, 345, 455. These are not stubs. |
| 4 | Mailbox not consumed by loop | **STALE.** `execution.rs` imports `AgentRuntimeInbox`; `native_cognit.rs:258` checks "mailbox turn limit exhausted." |
| 5 | Context fork incomplete | **PARTIALLY STALE.** `context_fork.rs` exists in the `agent_control` directory. `candidate_projection.rs` handles Agora projection. |
| 6 | Tool governance bypass | **NEEDS VERIFICATION.** `GovernedCapabilityInvoker` exists at `governed_capability.rs:109`. Whether the full governance path is exercised requires confirmation. |
| 7 | Result destroyed immediately | **STALE.** `SqliteAgentRunRepository` at `sqlite_repository.rs` persists agent runs with a durable result lifecycle. |
| 8 | Agent definition not honored (hard-coded 20 iterations) | **PARTIALLY STALE.** `native_cognit.rs` has configurable max iterations from `AgentProfile`. |
| 9 | Pi integration treats stdout as opaque | **STILL ACCURATE.** `PiRuntime` at `pi.rs` collects stdout as the final answer. |
| 10 | No concurrent execution | **STALE.** `admission.rs` handles budget-controlled concurrent spawn admission. |

**Summary:** 7 of 10 gap claims are stale (fully resolved in code). 2 are partially stale
(context fork, agent definition). 1 remains accurate (Pi stdout opacity). All 141
implementation-phase task checkboxes remain unchecked in this document; an
implementer should re-validate against current code before starting any phase.

## 1. Goal

Replace the current duplicated sub-agent execution path with one Executive-managed Agent Process path that supports:

```text
spawn
wait
send
cancel
inspect/list
```

Every child Agent must run through an approved Cognit Harness or external supervised runtime, inherit only a bounded context projection, have explicit resources and permissions, and produce a durable structured result.

The end state is not “Aletheon becomes Codex.” The end state is:

```text
Aletheon Executive owns Agent Process lifecycle.
Cognit owns native reasoning Harnesses.
Corpus owns executable capabilities.
Agora owns shared working projections.
Mnemosyne owns scoped experience and memory.
Pi or other coding engines are replaceable SubAgentRuntime implementations.
```

### 1.1 Role in the conscious system

SubAgents are not merely background workers. They provide cognitive plurality:
specialized, partially independent processors can investigate different
hypotheses, detect conflicts and return evidence into one globally integrated
root workspace.

```text
root Dasein concern / Agora broadcast
    -> AgentControl selects and scopes specialist processors
    -> SubAgents reason and act through governed capabilities
    -> child evidence/hypotheses/results become Agora candidates
    -> root Agora selects what becomes globally available
    -> root Dasein integrates selected content
    -> Mnemosyne records child-scoped experience and reviewed promotion
```

Default subject rule:

```text
one root Aletheon identity = one DaseinCore + one root Agora episode
ordinary SubAgents = local cognitive processors of that root identity
```

A child Process, model session or private memory scope does not by itself create
another self. An independently persistent subject would require an explicit
identity configuration, its own Dasein ledger, root Agora workspace, memory
authority, continuity policy and lifecycle/welfare review.

Detailed conscious-system integration:

- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md`

## 2. Current implementation truth

Aletheon already has more than a UI-only sub-agent stub.

### 2.1 Existing capabilities

- `SubAgentSpawner` registers ProcessTable and OperationTable entries.
- Every tracked child receives a process ID, operation ID and mailbox.
- Parent process space can be forked into a child space.
- RuntimeRegistry supports runtime selection.
- `SubAgentRuntime` supports structured attempt results.
- SupervisorTree and restart policies exist.
- Agent definitions include prompt, tools, model and iteration configuration.
- `AgentTool` can execute a real LLM/tool loop.
- TUI receives sub-agent status handles.

Primary anchors:

- `crates/executive/src/core/sub_agent.rs`
- `crates/executive/src/core/runtime_registry.rs`
- `crates/corpus/src/tools/tools/agent_tool.rs`
- `crates/executive/src/impl/daemon/handler/init.rs`
- `crates/kernel/src/process/`
- `crates/kernel/src/operation/`
- `crates/kernel/src/supervision/`
- `crates/fabric/src/ipc/mailbox.rs`

### 2.2 Blocking gaps

| Gap | Current behavior | Required behavior |
|---|---|---|
| Duplicate Harness | `init.rs` contains a separate hard-coded 20-step LLM loop | Native children use Cognit Harness through a runtime adapter |
| Blocking delegation | `AgentTool::execute` waits inline for completion | Spawn and wait are separate control operations; compatibility wait remains optional |
| Cancellation | tracked operation has a cancellation stub | The real LLM/tool loop observes the same token |
| Context fork | Process Space is forked, but the inline loop starts with only system/user messages | Explicit bounded ContextProjection is materialized into the child session |
| Tool governance | inline loop calls registry tools directly | Tool calls pass through the same Executive permission/approval path |
| Mailbox | mailbox exists but the execution loop does not consume it | send/interrupt messages reach the live Agent session |
| Result lifecycle | child is destroyed immediately after inline completion | terminal result/status remains queryable and restart-safe |
| Agent definition | model and `max_iterations` are not honored; loop uses 20 | resolved profile config controls runtime and Harness |
| Agora | child result is returned as a tool string | progress/evidence/result are projected into Agora trace |
| Memory | child has no explicit memory scope | Agent/Task memory isolation and controlled promotion |
| Concurrency | no shared execution admission for production AgentTool path | root-tree count and active execution limits |

The current system is best described as **synchronous delegated child execution**, not yet a complete collaborative multi-agent runtime.

## 3. Lessons to adopt from Codex

Codex provides useful control-plane patterns.

### 3.1 One control object per root Agent tree

Codex shares one `AgentControl` across a root thread and its descendants. It owns:

- registry scoped to the root tree;
- spawn reservations;
- active execution limits;
- rollout budget;
- status subscriptions;
- send/interrupt/list operations;
- persistent parent-child edges.

Aletheon should implement the same responsibility in Executive, but use existing kernel Process/Operation/Space primitives instead of copying Codex thread internals.

### 3.2 A child Agent is a session, not a callback

Codex spawns an independent child thread/session. Aletheon must similarly create an `AgentSession` backed by:

- Agent Process;
- Operation Scope;
- selected runtime/Harness;
- mailbox;
- context projection;
- policy and resource lease;
- durable result/status.

The current `ExecuteSubAgentFn` callback is only a temporary compatibility seam.

### 3.3 Context fork is filtered

Codex supports full-history or last-N-turn fork modes and filters raw reasoning/tool events during child creation.

Aletheon should use stricter modes:

```rust
pub enum AgentContextFork {
    None,
    LastTurns(usize),
    SelectedProjection,
}
```

Default to `SelectedProjection`, not full raw history. Copy:

- user intent and constraints;
- current Goal frame;
- approved memory projection;
- relevant Agora evidence;
- environment and working directory policy.

Do not copy:

- hidden reasoning;
- unbounded tool output;
- credentials;
- unrelated session history;
- parent-only approvals or capabilities.

### 3.4 Reserve before spawning

Codex uses a reservation object whose Drop path releases capacity if creation fails. Aletheon should add equivalent atomic admission around:

- Agent count;
- execution slot;
- token/cost budget;
- worktree/storage quota;
- Process/Operation creation.

Partial creation must not leave process, mailbox, worktree or budget leaks.

### 3.5 Status and edges are persistent

Codex stores thread spawn edges and can restore identities without immediately reopening every runtime.

Aletheon should persist Agent Run metadata and parent-child edges separately from live in-memory process handles.

### 3.6 Pi adoption decision: use it as a coding runtime, not the Agent kernel

Pi is useful to Aletheon, but at one deliberately narrow layer:

```text
Aletheon Goal / root Agent
    -> Executive AgentControl + policy + budget
    -> Kernel Process / Operation / cancellation
    -> PiRuntime adapter
    -> sandboxed Pi coding process in an isolated worktree
    -> structured events, diff and verification evidence
    -> Agora candidate + reviewed Mnemosyne promotion
```

The decision is:

| Option | Decision | Reason |
|---|---|---|
| Replace Cognit with Pi Agent Core | Reject | Creates competing Harness, Session, context and memory authorities |
| Embed Pi's TypeScript SDK in the Rust process | Reject initially | Tightens Node/TypeScript coupling and weakens process isolation |
| One-shot Pi coding job | Keep | Good for bounded implementation, refactor, test and repository-analysis tasks |
| Persistent Pi RPC child | Add later | Enables send, steer, follow-up, abort and structured live progress |
| Pi extensions/packages as trusted Aletheon plugins | Deny by default | They execute code and are outside Aletheon's capability authority |

Current Aletheon already implements much of the one-shot path in
`crates/executive/src/impl/runtime/pi.rs`:

- stable runtime ID `pi-coder` and opt-in registration;
- fixed executable/arguments and fail-closed sandbox capability checks;
- disabled network, isolated Git worktree and allowed/forbidden path policy;
- timeout, cancellation, bounded stdout/stderr and descendant process control;
- changed-file inspection, diff hashing and structured attempt evidence;
- retained worktree for verification and explicit approval/apply.

This means Pi does not need to be integrated from zero. The next work is to
harden the real protocol boundary. The current configuration example selects
`--mode json`, while `PiRuntime` treats the complete stdout stream as the final
`RuntimeResult.output`. Upstream JSON mode emits a JSONL session/event stream,
so Aletheon must parse and validate it instead of storing it as an opaque final
answer. Existing fixture scripts verify the process wrapper but do not prove
compatibility with a pinned real Pi release.

Implement two explicit adapter modes:

1. **`PiJobRuntime`:** one process per coding attempt. Parse JSON events,
   derive the terminal assistant result, tool evidence and usage, and reject
   malformed/truncated streams. Use this first.
2. **`PiRpcRuntime`:** one supervised RPC process per resident child Agent.
   Translate Aletheon `send`, `steer`, `follow_up`, `cancel` and status calls to
   strict LF-delimited JSONL. Use this only after AgentControl and mailbox
   semantics are stable.

Both modes must:

- pin and record the Pi package version/build identity;
- run a real-Pi contract test in CI, not only a shell fixture;
- disable session persistence or redirect it into child-scoped storage;
- ignore project/global extensions, packages and context resources unless an
  explicit Aletheon allowlist authorizes them;
- keep provider credentials out of the worktree and result evidence;
- translate Pi events into Aletheon Items/candidates instead of exposing Pi's
  internal session as a second source of truth;
- fail closed on protocol drift, unknown privileged events or isolation loss.

Upstream references:

- [Pi coding-agent README](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)
- [Pi JSON event stream](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/json.md)
- [Pi RPC protocol](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/rpc.md)

## 4. Target architecture

```text
Root Dasein + Agora broadcast
    -> Executive ConsciousCoreCoordinator
       -> AgentControlService
          +-- admission/concurrency reservation
          +-- ProcessTable / OperationTable / Space fork
          +-- AgentRunRepository
          +-- RuntimeRegistry
          +-- mailbox / status stream
          -> SubAgentRuntime
             +-- NativeCognitRuntime -> Cognit Harness
             +-- PiCodingRuntime     -> isolated worktree process
             +-- future runtimes
          -> child-scoped experience in Mnemosyne
          -> typed AgentResult/Progress candidates
    -> root Agora competition and broadcast
    -> root Dasein integration
```

## 5. Contracts

### 5.1 Shared Fabric contracts

Create or consolidate these under Fabric so Corpus can expose tools without depending on Executive.

Suggested files:

- Create `crates/fabric/src/types/agent_control.rs`
- Modify `crates/fabric/src/types/mod.rs`
- Modify `crates/fabric/src/lib.rs`

```rust
pub struct AgentSpawnRequest {
    pub profile: AgentProfileId,
    pub runtime_id: Option<RuntimeId>,
    pub parent_process_id: ProcessId,
    pub parent_operation_id: Option<OperationId>,
    pub task: String,
    pub context_fork: AgentContextFork,
    pub scope: AgentExecutionScope,
    pub budget: AgentBudget,
    pub restart_policy: RestartPolicySpec,
}

pub struct AgentHandle {
    pub agent_id: AgentId,
    pub process_id: ProcessId,
    pub operation_id: OperationId,
    pub status: AgentStatus,
}

pub struct AgentResult {
    pub status: AgentTerminalStatus,
    pub output: String,
    pub evidence: Vec<AttemptEvidence>,
    pub artifacts: Vec<ArtifactRef>,
    pub usage: AttemptUsage,
    pub memory_candidates: Vec<MemoryCandidateRef>,
    pub exit_reason: AgentExitReason,
}
```

All strings and collections require hard size/count limits before persistence or prompt injection.

### 5.2 Control port

Define a port rather than allowing Corpus to call Executive concrete types:

```rust
#[async_trait]
pub trait AgentControlPort: Send + Sync {
    async fn spawn(&self, request: AgentSpawnRequest) -> anyhow::Result<AgentHandle>;
    async fn wait(&self, agent: AgentId, timeout: Duration) -> anyhow::Result<AgentResult>;
    async fn send(&self, agent: AgentId, message: AgentMessage) -> anyhow::Result<MessageReceipt>;
    async fn cancel(&self, agent: AgentId, reason: CancelReason) -> anyhow::Result<()>;
    async fn inspect(&self, agent: AgentId) -> anyhow::Result<AgentSnapshot>;
    async fn list(&self, root: AgentId) -> anyhow::Result<Vec<AgentSnapshot>>;
}
```

Executive implements this port. Corpus tools receive `Arc<dyn AgentControlPort>`.

### 5.3 Runtime contract evolution

Do not discard the current M3 `run`/`run_attempt` compatibility immediately. Add a richer default method:

```rust
#[async_trait]
pub trait SubAgentRuntime: Send + Sync {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>;

    async fn run_attempt(
        &self,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure>;

    async fn run_process(
        &self,
        request: SubAgentExecutionRequest,
        events: Arc<dyn AgentEventSink>,
        cancel: CancellationToken,
    ) -> Result<AgentResult, RuntimeFailure> {
        // Compatibility adapter to run_attempt.
    }
}
```

New native and Pi runtimes override `run_process`. Old test runtimes remain buildable during migration.

## 6. Ownership rules

| Responsibility | Owner |
|---|---|
| Agent lifecycle, admission, budget, cancellation | Executive |
| Reasoning loop/Harness | Cognit |
| Tool implementation | Corpus |
| Tool permission and approval decision | Executive policy boundary |
| Shared task/evidence projection | Agora |
| Agent experience and promoted memory | Mnemosyne |
| Root identity, values, self attribution and child-result interpretation | Dasein |
| Conscious-core recurrence and processor registration | Executive ConsciousCoreCoordinator |
| Process/Operation/Space primitives | Kernel |

Forbidden dependencies:

- Cognit must not mutate ProcessTable directly.
- Corpus must not depend on Executive concrete implementations.
- Kernel must not know provider, model, Pi or GBrain.
- Sub-agent recalled text must not mutate Dasein.
- SubAgents must submit root-visible information as Agora candidates rather than
  mutating the root workspace directly.
- ordinary child Agents must not instantiate or claim a separate persistent
  identity implicitly.
- Pi must not become the primary Aletheon Cognit engine.

## 7. Implementation phases

### Phase A1 — Freeze the current sub-agent vertical slice

**Purpose:** Establish behavior before removing the inline loop.

**Files:**

- Create `crates/executive/tests/subagent_production_baseline.rs`
- Extend existing AgentTool/SubAgentSpawner tests

**Tasks:**

- [ ] Boot with one test Agent definition.
- [ ] Delegate one task through the actual registered AgentTool.
- [ ] Assert ProcessTable/OperationTable/mailbox registration.
- [ ] Assert allowed-tool filtering.
- [ ] Assert terminal success and failure mapping.
- [ ] Add target-behavior cancellation coverage as `#[ignore = "known A4 gap"]`; unignore it when the real Harness consumes the token.
- [ ] Add ignored target coverage for model/max-iterations enforcement and unignore it in A4. Do not commit a red default test suite.
- [ ] Prove child process Space inherits selected parent bindings.

**Commit gate:**

```text
test(executive): lock subagent production baseline
```

### Phase A2 — Add shared Agent control contracts

**Purpose:** Create the stable boundary before changing implementation.

**Files:**

- Create `crates/fabric/src/types/agent_control.rs`
- Modify Fabric exports
- Add serialization and bounds tests

**Tasks:**

- [ ] Define request, handle, status, snapshot, message and result types.
- [ ] Define `AgentContextFork` and explicit budgets.
- [ ] Define `AgentControlPort`.
- [ ] Reuse existing `AgentId`, `ProcessId`, `OperationId`, `RuntimeId` and attempt types.
- [ ] Avoid parallel duplicate ID/status enums.
- [ ] Add maximum task/output/evidence/artifact counts.

**Commit gate:**

```text
feat(fabric): define agent control contracts
```

### Phase A3 — Implement Executive AgentControlService

**Purpose:** Centralize lifecycle operations over existing kernel primitives.

**Files:**

- Create `crates/executive/src/service/agent_control/mod.rs`
- Create `crates/executive/src/service/agent_control/admission.rs`
- Create `crates/executive/src/service/agent_control/repository.rs`
- Create `crates/executive/src/service/agent_control/context_fork.rs`
- Refactor `crates/executive/src/core/sub_agent.rs`

**Tasks:**

- [ ] Reserve capacity before creating any process resources.
- [ ] Spawn Process, Operation, Space and mailbox transactionally/compensatingly.
- [ ] Persist parent-child edge and initial status.
- [ ] Resolve runtime before committing the reservation.
- [ ] Start exactly one runtime task owned by the OperationScope.
- [ ] Connect the OperationScope CancellationToken to the real runtime.
- [ ] Implement wait with status subscription, not polling loops.
- [ ] Keep terminal result/status queryable after runtime completion.
- [ ] Add retention cleanup separate from process completion.
- [ ] Keep SupervisorTree for crash restart, not provider retry policy.

Suggested persistence:

```sql
CREATE TABLE agent_runs (
    agent_id TEXT PRIMARY KEY,
    root_agent_id TEXT NOT NULL,
    parent_agent_id TEXT,
    process_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    runtime_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    status TEXT NOT NULL,
    request_json TEXT NOT NULL,
    result_json TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    ended_at TEXT,
    last_error TEXT
);

CREATE INDEX idx_agent_runs_root_status
ON agent_runs(root_agent_id, status, created_at);
```

**Commit gate:**

```text
feat(executive): centralize agent process control
```

### Phase A4 — Build NativeCognitRuntime from the existing Harness

**Purpose:** Remove the duplicate LLM loop from `init.rs`.

**Files:**

- Create `crates/executive/src/impl/runtime/native_cognit.rs`
- Modify `crates/executive/src/impl/runtime/mod.rs`
- Modify `crates/executive/src/impl/daemon/handler/init.rs`
- Reuse `crates/cognit/src/harness/*`
- Reuse `crates/executive/src/service/harness_factory.rs`

**Tasks:**

- [ ] Build one `CognitiveSession`/Harness per child Agent process.
- [ ] Apply Agent profile system prompt, model/provider, tool list and max iterations.
- [ ] Seed the child with `ContextProjection`, not raw parent history.
- [ ] Route tool execution through the governed Executive tool path.
- [ ] Connect cancellation to Harness interrupt and tool cancellation.
- [ ] Aggregate usage/evidence into `AgentResult`.
- [ ] Emit progress, tool and terminal events through `AgentEventSink`.
- [ ] Register as `RuntimeId("native-cognit")`.
- [ ] Keep `NativeCognitEngine v0` as the default authority.
- [ ] Delete the inline 20-step loop only after equivalent tests pass.

**Required parity tests:**

- [ ] final text response;
- [ ] one and multiple tool calls;
- [ ] unknown tool rejection;
- [ ] max-iteration exhaustion;
- [ ] provider failure;
- [ ] cancellation during provider call and tool call;
- [ ] profile model/max-iteration enforcement;
- [ ] no tool outside allow-list.

**Commit gates:**

```text
feat(executive): run subagents through Native Cognit harness
refactor(executive): remove inline AgentTool reasoning loop
```

### Phase A5 — Convert AgentTool into a thin control client

**Purpose:** Stop the tool from owning execution.

**Files:**

- Modify `crates/corpus/src/tools/tools/agent_tool.rs`
- Add dedicated Agent control tools or subcommands

**Tasks:**

- [ ] Replace `ExecuteSubAgentFn` with `Arc<dyn AgentControlPort>`.
- [ ] Keep existing `agent` tool as compatibility `spawn + wait` behavior.
- [ ] Add explicit asynchronous operations:

```text
agent_spawn
agent_wait
agent_send
agent_cancel
agent_list
```

- [ ] Return Agent IDs and structured status, not only free-form strings.
- [ ] Require explicit timeout for wait.
- [ ] Make repeated wait/inspect idempotent.
- [ ] Reject cross-root access unless policy allows it.
- [ ] Keep role/tool/model config in one shared `AgentProfile`, not mirrored types.

**Commit gate:**

```text
refactor(corpus): make agent tools use Executive control port
```

### Phase A6 — Make context and conscious-workspace participation real

**Purpose:** Give child Agents useful isolated context and observable results.

**Files:**

- Extend `crates/agora/src/`
- Add Executive ContextFork builder
- Modify child session creation

**Tasks:**

- [ ] Add a bounded `AgentContextProjection`.
- [ ] Support `None`, `LastTurns(n)` and `SelectedProjection`.
- [ ] Default to selected Goal/constraints/memory/evidence.
- [ ] Never copy hidden reasoning or raw unbounded tool output.
- [ ] Allocate one child Agora namespace/trace.
- [ ] Append child progress and evidence to that trace.
- [ ] Subscribe the child only to broadcasts allowed by its task, Space and
      visibility scope.
- [ ] Convert progress, evidence, hypotheses, criticism and terminal results
      into typed `WorkspaceCandidate` values.
- [ ] Project candidates to the parent Agora through ConsciousCoreCoordinator;
      never directly commit them as globally selected content.
- [ ] Preserve broadcast epoch/content IDs in child requests and responses.
- [ ] Apply source quotas, TTL and anti-monopoly policy to Agent candidates.
- [ ] Preserve artifact references instead of copying large contents.
- [ ] Mark recalled/child content as data, not instructions.

**Commit gate:**

```text
feat(agora): project isolated subagent context and results
```

### Phase A7 — Activate mailbox communication

**Purpose:** Turn the existing mailbox registry into runtime collaboration.

**Tasks:**

- [ ] Give each Agent session a mailbox receive loop or multiplexed control channel.
- [ ] Define schemas for input, progress, result, signal and request/response.
- [ ] `send` may optionally trigger a new child turn.
- [ ] Interrupt/cancel signals remain high priority.
- [ ] Apply mailbox capacity and backpressure.
- [ ] Reject messages after terminal state except inspect/audit access.
- [ ] Persist required messages or result references for restart recovery.
- [ ] Test parent-child, sibling through parent policy, and unknown target behavior.

**Commit gate:**

```text
feat(executive): connect agent mailboxes to live sessions
```

### Phase A8 — Add multi-agent admission and budgets

**Purpose:** Permit concurrency without uncontrolled fan-out.

**Tasks:**

- [ ] Configure max total Agents per root tree.
- [ ] Configure max concurrently executing Agents.
- [ ] Configure maximum depth.
- [ ] Share a root rollout/token/cost budget.
- [ ] Reserve capacity before spawn and release through RAII/Drop.
- [ ] Prevent recursive delegation for memory workers and other internal roles.
- [ ] Apply worktree/storage quota before Pi spawn.
- [ ] Distinguish queued, running and resident-but-idle Agents.
- [ ] Add fairness policy for sibling Agents.

**Commit gate:**

```text
feat(executive): bound multiagent execution and rollout budgets
```

### Phase A9 — Add scoped memory and result promotion

**Dependency:** Mnemosyne unified memory Phases M2-M4.

**Tasks:**

- [ ] Give child process an Agent/Task MemoryScope.
- [ ] Fork a bounded approved parent MemoryProjection.
- [ ] Record child actions/results as child experiences.
- [ ] Return memory candidates in `AgentResult`.
- [ ] Require parent review or consolidation before promotion.
- [ ] Never let one child directly update Global/Core/Dasein state.
- [ ] Preserve child Agent ID and source event IDs after promotion.
- [ ] Record which root Agora broadcast triggered the child work and which
      child candidate was later selected.
- [ ] Require root Agora selection plus parent/consolidator policy before a
      child result can influence Dasein narrative or Session/Global memory.
- [ ] Keep unselected child work available for audit and scoped reuse without
      pretending it was globally experienced by the root.

**Commit gate:**

```text
feat(executive): isolate and promote subagent experience
```

### Phase A10 — Restart recovery and cleanup

**Purpose:** Make Agent Process state durable enough for long-running Goals.

**Tasks:**

- [ ] On daemon restart, load open AgentRun records and parent-child edges.
- [ ] Mark non-resumable in-flight native provider calls as interrupted, never silently replay them.
- [ ] Resume only runtimes with an explicit resumable contract.
- [ ] Reclaim expired execution reservations and leases.
- [ ] Preserve terminal result metadata after live runtime eviction.
- [ ] Clean worktrees only after verified terminal state.
- [ ] Retain audit/result rows according to storage policy.

**Commit gate:**

```text
feat(executive): recover durable agent process metadata
```

## 8. First usable multi-agent vertical slice

The first release should implement only this scenario:

```text
root Native Cognit
    -> spawn researcher and coder concurrently
    -> each receives selected context and tool policy
    -> parent can list/send/cancel/wait
    -> results appear in Agora
    -> reviewer validates outputs
    -> selected facts become Mnemosyne candidates
```

Do not begin agent societies, voting, auctions, decentralized planning or arbitrary recursive spawning before this vertical slice is reliable.

## 9. Required tests

### 9.1 Lifecycle

- [ ] spawn success/failure compensation;
- [ ] state transition legality;
- [ ] wait before and after completion;
- [ ] cancel before start, during LLM, during tool, after completion;
- [ ] terminal result retention;
- [ ] daemon restart metadata recovery.

### 9.2 Context and permissions

- [ ] selected context only;
- [ ] no hidden reasoning/tool-output leak;
- [ ] no cross-session memory leak;
- [ ] child tool allow-list;
- [ ] approval path cannot be bypassed;
- [ ] working directory and writable roots are enforced.

### 9.3 Concurrency and budgets

- [ ] two children run concurrently;
- [ ] max thread/depth/execution limits;
- [ ] reservation released on every spawn failure point;
- [ ] token/cost budget settlement;
- [ ] recursive internal worker delegation rejected.

### 9.4 Communication

- [ ] parent-child send;
- [ ] progress/status subscription;
- [ ] mailbox backpressure;
- [ ] cancel signal priority;
- [ ] terminal mailbox behavior;
- [ ] no cross-root messaging without capability.

### 9.5 Runtime parity

- [ ] Native Cognit basic dialogue remains operational.
- [ ] Pi remains optional and supervised.
- [ ] runtime selection is configuration, not provider-brand branching.
- [ ] no duplicate LLM loop remains in daemon bootstrap.
- [ ] A pinned real Pi build passes one-shot stdin/prompt and JSONL protocol
      contract fixtures.
- [ ] Pi JSON mode is parsed into typed events and one terminal result rather
      than persisted as opaque stdout.
- [ ] Project-local Pi extensions, packages and context are ignored unless
      explicitly allowlisted by Aletheon policy.
- [ ] Persistent Pi RPC, if enabled, maps send/steer/follow-up/abort to the
      authoritative Agent Process and mailbox lifecycle.

## 10. Observability

Add sanitized metrics and trace events:

```text
agent_spawn_total{runtime,profile,result}
agent_active{root,runtime}
agent_execution_wait_ms
agent_turn_latency_ms{runtime}
agent_cancel_total{phase,reason}
agent_mailbox_depth{agent}
agent_message_rejected_total{reason}
agent_budget_used{kind}
agent_result_bytes{runtime}
agent_context_projection_bytes{mode}
```

Do not log prompts, secrets, full tool outputs or confidential memory.

## 11. Release gates

The unified Harness is complete only when:

- [ ] `init.rs` contains no independent sub-agent LLM loop.
- [ ] Native children use Cognit Harness through `NativeCognitRuntime`.
- [ ] AgentTool depends only on the Agent control port.
- [ ] spawn/wait/send/cancel/list operate on real Agent Processes.
- [ ] cancellation reaches the actual provider/tool work.
- [ ] model, max iterations and tool policy from Agent profile are honored.
- [ ] child context and memory are isolated and bounded.
- [ ] mailbox communication reaches the live Agent session.
- [ ] terminal results survive live process cleanup.
- [ ] two children can run concurrently under shared limits.
- [ ] Pi remains optional; Aletheon Native Cognit remains primary.
- [ ] Pi's pinned build identity and protocol version are attached to every
      coding attempt.
- [ ] Malformed or incompatible Pi event streams fail closed before result
      promotion or approval.

## 12. Suggested verification commands

```bash
cargo fmt --all -- --check
cargo test -p fabric -- agent_control
cargo test -p executive --test subagent_production_baseline
cargo test -p executive --test runtime_registry
cargo test -p executive --test agent_process_test
cargo test -p executive --test process_messaging
cargo test -p executive --test supervision
cargo test -p executive --test budget_quota_lease
cargo test -p corpus -- agent_tool
cargo test --workspace
cargo build --workspace
```

## 13. Recommended implementation batches

```text
Batch 1: A1-A3  -> contracts and Executive control plane
Batch 2: A4-A5  -> unified Native Cognit Harness and thin AgentTool
Batch 3: A6-A7  -> isolated context, Agora and mailbox
Batch 4: A8     -> bounded concurrency and budgets
Batch 5: A9-A10 -> scoped memory and restart recovery
```

Start with Batch 1 and Batch 2. They remove the most dangerous duplication while preserving the current externally visible AgentTool behavior.
