# Goal Runtime Architecture

> **Status:** Proposed  
> **Purpose:** Convert user intent into persistent, iterative and supervised execution.

## 1. Goal Is a Persistent Object

```rust
pub struct Goal {
    pub id: GoalId,
    pub parent: Option<GoalId>,
    pub owner: PrincipalId,
    pub intent: GoalIntent,
    pub desired_state: DesiredState,
    pub constraints: Vec<Constraint>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub state: GoalState,
    pub priority: Priority,
    pub budget: GoalBudget,
    pub plan: Option<Plan>,
    pub progress: GoalProgress,
    pub evidence: Vec<EvidenceRef>,
    pub blockers: Vec<Blocker>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deadline: Option<Timestamp>,
}
```

```rust
pub enum GoalState {
    Draft,
    Clarifying,
    Planned,
    Running,
    Verifying,
    Blocked,
    AwaitingHuman,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}
```

## 2. Intent Compilation

Input:

```text
/goal Integrate Pi as a coding subagent with isolated worktrees,
timeouts, structured reports and Native Cognit review.
```

Output:

```yaml
objective: Integrate Pi as a supervised coding subagent

desired_state:
  - Aletheon can spawn Pi
  - Pi runs in an isolated worktree
  - timeout and cancellation work
  - stdout and stderr are captured
  - result becomes SubagentReport
  - Native Cognit reviews the result

constraints:
  - Native Cognit remains primary
  - Pi cannot modify Dasein
  - core crates do not depend on Pi
  - Pi cannot modify the main worktree directly

acceptance_criteria:
  - cargo check passes
  - integration tests pass
  - one real delegated task succeeds
  - failed execution leaves the main workspace unchanged
```

The original intent remains immutable. Plans may change; the goal specification may not silently drift.

## 3. Goal Supervisor

```rust
#[async_trait]
pub trait GoalSupervisor {
    async fn create_goal(
        &self,
        specification: GoalSpecification,
    ) -> Result<GoalId>;

    async fn tick(
        &self,
        goal_id: GoalId,
    ) -> Result<GoalTransition>;

    async fn suspend(&self, goal_id: GoalId) -> Result<()>;
    async fn resume(&self, goal_id: GoalId) -> Result<()>;
    async fn cancel(&self, goal_id: GoalId) -> Result<()>;
}
```

Each `tick()` advances a bounded amount of work.

## 4. Goal Cycle

```text
Observe
  ↓
Evaluate
  ↓
Select
  ↓
Execute
  ↓
Verify
  ↓
Record
  ↓
Replan
```

The system must not use an unbounded `while not done` model loop.

## 5. Plan Graph

```rust
pub struct Plan {
    pub version: u32,
    pub tasks: HashMap<TaskId, PlannedTask>,
    pub created_from: GoalSnapshot,
}
```

```rust
pub struct PlannedTask {
    pub id: TaskId,
    pub objective: String,
    pub dependencies: Vec<TaskId>,
    pub state: TaskState,
    pub executor: ExecutorPreference,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub attempts: Vec<AttemptId>,
}
```

Example:

```text
Goal
├── T1 Define SubagentTask
├── T2 Define SubagentReport
├── T3 Implement Pi adapter
│      depends_on: T1, T2
├── T4 Add worktree isolation
│      depends_on: T3
├── T5 Add timeout and cancellation
│      depends_on: T3
├── T6 Add integration tests
│      depends_on: T4, T5
└── T7 Add Native review workflow
       depends_on: T2, T6
```

## 6. Attempts

```rust
pub struct Attempt {
    pub id: AttemptId,
    pub task_id: TaskId,
    pub executor: ExecutorRef,
    pub input: TaskContext,
    pub result: AttemptResult,
    pub evidence: Vec<EvidenceRef>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
}
```

A task may have multiple attempts, each with independent evidence.

## 7. Failure Classification

```rust
pub enum FailureClass {
    Compilation,
    TestFailure,
    PermissionDenied,
    Timeout,
    MissingDependency,
    InvalidAssumption,
    ArchitectureViolation,
    ToolFailure,
    ContextInsufficient,
    RepeatedFailure,
}
```

Policy examples:

```text
Compilation
→ DeepSeek retries with compiler evidence

ArchitectureViolation
→ GPT or Opus replans or reviews

RepeatedFailure
→ shrink the task or change executor

ContextInsufficient
→ query Mnemosyne or ask the user

PermissionDenied
→ Executive checks capabilities
```

## 8. Retry and Escalation

Recommended default:

```text
Attempt 1 failed
→ same worker with evidence

Same failure again
→ change strategy or shrink task

Third repeated failure
→ GPT or Opus root-cause analysis

Still unresolved
→ Blocked or AwaitingHuman
```

No infinite retries.

## 9. Verification Gates

Code task example:

```text
implementation
    ↓
format
    ↓
compile
    ↓
unit tests
    ↓
integration tests
    ↓
diff scope check
    ↓
architecture review
    ↓
accept
```

```rust
pub struct VerificationReport {
    pub passed: bool,
    pub checks: Vec<VerificationCheck>,
    pub artifacts: Vec<ArtifactRef>,
    pub risks: Vec<String>,
}
```

## 10. Model Routing

```text
Native Cognit
= control, intent ownership, orchestration and final decision

DeepSeek
= low-cost iterative worker

Pi
= specialized coding subagent

GPT / Opus
= planner, architect, reviewer and escalation model
```

```rust
pub enum CognitiveRole {
    IntentCompiler,
    Planner,
    Worker,
    Reviewer,
    Debugger,
    Summarizer,
    Verifier,
}
```

## 11. Goal Frame

```rust
pub struct GoalFrame {
    pub original_intent: String,
    pub desired_state: DesiredState,
    pub constraints: Vec<Constraint>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub current_plan: PlanSummary,
    pub current_task: PlannedTask,
    pub recent_attempts: Vec<AttemptSummary>,
    pub relevant_memories: Vec<MemoryProjection>,
    pub remaining_budget: GoalBudget,
}
```

Every worker receives the Goal Frame so the original intent remains visible.

## 12. Agora and Mnemosyne

Agora stores active Goal state:

```text
GoalFrame
current task
recent observations
active blockers
subagent reports
verification results
pending approvals
```

Mnemosyne stores durable history:

```text
Goal specification
plan versions
attempts
failures
decisions
outcomes
lessons
procedures
```

## 13. Commands

```text
/goal <objective>
/goals
/status <goal-id>
/pause <goal-id>
/resume <goal-id>
/cancel <goal-id>
/approve <request-id>
/reject <request-id>
```

## 14. Safety

Every Goal requires:

- budget limit;
- time limit;
- attempt limit;
- capability boundary;
- workspace boundary;
- pause and cancellation;
- audit logs;
- approval policy;
- completion criteria;
- escalation policy.

## 15. MVP

Implement `Single Active Goal Runtime v0`:

- one active persistent Goal;
- task list or small DAG;
- DeepSeek worker;
- Pi subagent;
- verification;
- three-attempt default;
- GPT/Opus escalation;
- pause, resume and cancel;
- Telegram progress;
- SQLite/Postgres persistence;
- completion summary.
