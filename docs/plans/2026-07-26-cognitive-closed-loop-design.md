# Cognitive Closed Loop Design

## Status

Approved direction, pending written-spec review before implementation planning.

## Objective

Make `cognit` the authoritative task-cognition engine for Aletheon. A turn must
understand the user's required method and deliverables, maintain explicit
knowledge and evidence state, verify important claims, and prove completion
before the runtime accepts a final answer.

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

### Explicitly deferred

- General natural-language theorem proving or universal factual verification.
- Broad redesign of Agora, Mnemosyne, Dasein, or the Conscious Workspace.
- Automatic mutation of prompts, policies, completion criteria, or boundaries.
- Production enforcement by Conscious Workspace; it remains in `Observe` while
  cognitive decision quality is measured.
- Benchmarking Pi, Codex, and Grok Build. The benchmark follows once the same
  completion/evidence contract can instrument every run.

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
    pub observations: Vec<Observation>,
    pub validations: Vec<ValidationRecord>,
    pub outstanding: Vec<Obligation>,
    pub completion_attempts: u32,
}
```

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

1. **Contract and evidence types** — no behavior change.
2. **Runtime receipt adapters and persistence** — observation only.
3. **Completion gate in shadow mode** — record would-reject decisions.
4. **Enforce required-action and terminal-evidence checks** — bounded recovery.
5. **Claim audit and grounded Dasein events** — deterministic claims only.
6. **Installed-runtime acceptance and regression documentation.**

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
