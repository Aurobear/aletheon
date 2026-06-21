# Agent Loop Redesign Design

**Date:** 2026-06-21
**Scope:** Redesign the Agent loop to fix cycle hanging, improve context recovery, and add tool execution safety

## Problem Statement

The current Agent implementation has three critical issues:

1. **Cycle Hanging** — Agent gets stuck in infinite loops, repeatedly calling tools without making progress
2. **Poor Context Recovery** — After restart, Agent loses conversation context and inefficiently searches memory
3. **Excessive Tool Calls** — No budget limits, Storm Breaker warnings are ignored, leading to tool call spam

### Root Cause Analysis

**Cycle Hanging:**
- No clear stopping conditions in the ReAct loop
- Agent doesn't track goals or progress
- Storm Breaker only warns, doesn't enforce

**Poor Context Recovery:**
- No automatic conversation summarization
- Memory search is untargeted and inefficient
- No smart context rebuilding on restart

**Excessive Tool Calls:**
- No tool call budget per turn
- No loop detection mechanism
- Agent doesn't reflect on progress periodically

## Solution: Agent Loop Redesign

### Core Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Agent Loop Redesign                       │
├─────────────────────────────────────────────────────────────┤
│  1. Goal Setting Phase                                       │
│     - Agent explicitly states goal                           │
│     - Decompose into sub-goals (max 3)                       │
│     - Set success criteria                                   │
├─────────────────────────────────────────────────────────────┤
│  2. Execution Phase (ReAct with Budget)                      │
│     - Reason: Based on goal and current state                │
│     - Act: Execute tool calls (with budget)                  │
│     - Observe: Observe results                               │
│     - [NEW] Progress Check: Check progress every 3 tool calls│
├─────────────────────────────────────────────────────────────┤
│  3. Reflection Checkpoint (every 5 tool calls)               │
│     - Summarize current progress                             │
│     - Assess proximity to goal                               │
│     - Decide: Continue / Adjust strategy / Stop              │
├─────────────────────────────────────────────────────────────┤
│  4. Termination Conditions                                   │
│     - Goal completed (success)                               │
│     - Tool budget exhausted (forced stop)                    │
│     - Timeout (forced stop)                                  │
│     - Loop detected (forced stop)                            │
└─────────────────────────────────────────────────────────────┘
```

### Component Design

#### 1. GoalTracker

**Location:** `crates/runtime/src/core/react_loop/goal_tracker.rs`

**Responsibilities:**
- Track current goal and sub-goals
- Evaluate goal completion
- Provide goal context for reasoning

**Interface:**
```rust
pub struct GoalTracker {
    current_goal: Option<Goal>,
    sub_goals: Vec<SubGoal>,
    success_criteria: Vec<String>,
}

pub struct Goal {
    pub description: String,
    pub created_at: Instant,
    pub status: GoalStatus,
}

pub enum GoalStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
}

impl GoalTracker {
    pub fn set_goal(&mut self, goal: String) -> Result<()>;
    pub fn add_sub_goal(&mut self, sub_goal: String) -> Result<()>;
    pub fn update_progress(&mut self, progress: ProgressUpdate) -> Result<()>;
    pub fn is_complete(&self) -> bool;
    pub fn get_context(&self) -> String;
}
```

#### 2. ToolBudget

**Location:** `crates/runtime/src/core/react_loop/tool_budget.rs`

**Responsibilities:**
- Manage tool call budget per turn
- Enforce budget limits
- Track tool call history

**Interface:**
```rust
pub struct ToolBudget {
    max_calls: usize,
    used_calls: usize,
    call_history: Vec<ToolCallRecord>,
}

pub struct ToolCallRecord {
    pub tool_name: String,
    pub timestamp: Instant,
    pub success: bool,
}

impl ToolBudget {
    pub fn new(max_calls: usize) -> Self;
    pub fn can_call(&self) -> bool;
    pub fn record_call(&mut self, record: ToolCallRecord) -> Result<()>;
    pub fn remaining(&self) -> usize;
    pub fn is_exhausted(&self) -> bool;
}
```

#### 3. ReflectionEngine

**Location:** `crates/runtime/src/core/react_loop/reflection.rs`

**Responsibilities:**
- Periodic reflection on progress
- Generate summaries
- Decide next actions

**Interface:**
```rust
pub struct ReflectionEngine {
    reflection_interval: usize,  // Reflect every N tool calls
    calls_since_reflection: usize,
}

impl ReflectionEngine {
    pub fn new(interval: usize) -> Self;
    pub fn should_reflect(&self) -> bool;
    pub fn reflect(&mut self, context: &ReflectionContext) -> ReflectionResult;
    pub fn reset(&mut self);
}

pub struct ReflectionContext {
    pub goal: Option<String>,
    pub recent_actions: Vec<String>,
    pub current_state: String,
}

pub struct ReflectionResult {
    pub summary: String,
    pub recommendation: ReflectionRecommendation,
}

pub enum ReflectionRecommendation {
    Continue,
    AdjustStrategy(String),
    Stop(StopReason),
}
```

#### 4. CircuitBreaker

**Location:** `crates/runtime/src/core/react_loop/circuit_breaker.rs`

**Responsibilities:**
- Detect infinite loops
- Detect repeated tool calls
- Force stop when patterns detected

**Interface:**
```rust
pub struct CircuitBreaker {
    recent_calls: VecDeque<ToolCallSignature>,
    max_repeats: usize,
    window_size: usize,
}

pub struct ToolCallSignature {
    pub tool_name: String,
    pub args_hash: u64,
    pub timestamp: Instant,
}

impl CircuitBreaker {
    pub fn new(max_repeats: usize, window_size: usize) -> Self;
    pub fn check(&mut self, call: &ToolCallSignature) -> CircuitBreakerStatus;
    pub fn reset(&mut self);
}

pub enum CircuitBreakerStatus {
    Ok,
    Warning(String),
    Tripped(String),
}
```

#### 5. ContextManager

**Location:** `crates/runtime/src/core/context_manager.rs`

**Responsibilities:**
- Manage layered context (Working Memory, Session Summary, Persistent Memory)
- Auto-summarize conversations
- Smart context recovery on restart

**Interface:**
```rust
pub struct ContextManager {
    working_memory: WorkingMemory,
    session_summary: Option<SessionSummary>,
    persistent_memory: MemoryRouter,
}

pub struct WorkingMemory {
    messages: VecDeque<Message>,
    max_messages: usize,
    current_goal: Option<String>,
}

pub struct SessionSummary {
    pub key_decisions: Vec<String>,
    pub completed_tasks: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub created_at: Instant,
}

impl ContextManager {
    pub fn new(memory_router: MemoryRouter) -> Self;
    pub fn add_message(&mut self, message: Message) -> Result<()>;
    pub fn should_summarize(&self) -> bool;
    pub fn generate_summary(&self) -> Result<SessionSummary>;
    pub fn recover_context(&mut self, query: &str) -> Result<ContextRecovery>;
    pub fn get_context_window(&self) -> Vec<Message>;
}

pub struct ContextRecovery {
    pub summary: Option<SessionSummary>,
    pub relevant_memories: Vec<Memory>,
    pub reconstructed_context: String,
}
```

### Integration with Existing Code

#### Modified Files

| File | Changes |
|------|---------|
| `crates/runtime/src/core/react_loop/mod.rs` | Integrate GoalTracker, ToolBudget, ReflectionEngine, CircuitBreaker |
| `crates/runtime/src/core/react_loop/step.rs` | Add progress checks and reflection checkpoints |
| `crates/runtime/src/core/react_loop/tool_exec.rs` | Add budget enforcement and loop detection |
| `crates/runtime/src/impl/daemon/handler/chat.rs` | Integrate ContextManager, improve context recovery |
| `crates/runtime/src/core/event_sink.rs` | Add new event types (GoalSet, Reflection, BudgetExceeded) |
| `crates/cognit/src/impl/llm/scheduler.rs` | Support context compression and summarization |

#### New Events

```rust
pub enum Event {
    // Existing events...
    GoalSet { goal: String, sub_goals: Vec<String> },
    Reflection { summary: String, recommendation: String },
    BudgetExceeded { used: usize, max: usize },
    CircuitBreakerTripped { reason: String },
    ContextRecovered { summary: Option<String>, memories_count: usize },
}
```

### Agent Loop Flow

```
┌─────────────────────────────────────────────────────────────┐
│                    New Agent Loop Flow                       │
├─────────────────────────────────────────────────────────────┤
│  1. Initialize                                              │
│     - Create GoalTracker, ToolBudget, ReflectionEngine,     │
│       CircuitBreaker, ContextManager                        │
│     - Recover context from previous session (if any)        │
├─────────────────────────────────────────────────────────────┤
│  2. Goal Setting                                            │
│     - Analyze user request                                  │
│     - Set main goal and sub-goals                           │
│     - Define success criteria                               │
├─────────────────────────────────────────────────────────────┤
│  3. Execution Loop                                          │
│     while !goal_complete && budget.remaining() > 0:         │
│       a. Reason: Based on goal, context, recent actions     │
│       b. Act: Execute tool call (check budget first)        │
│       c. Observe: Process tool result                       │
│       d. Check: Every 3 steps, check progress               │
│       e. Reflect: Every N steps, full reflection            │
│       f. Circuit Break: Check for loops after each call     │
├─────────────────────────────────────────────────────────────┤
│  4. Termination                                             │
│     - Goal completed → Success response                     │
│     - Budget exhausted → Partial response + warning         │
│     - Timeout → Emergency stop + state save                 │
│     - Loop detected → Forced stop + explanation             │
├─────────────────────────────────────────────────────────────┤
│  5. Post-Processing                                         │
│     - Generate turn summary                                 │
│     - Update persistent memory                              │
│     - Save context for next session                         │
└─────────────────────────────────────────────────────────────┘
```

### Configuration

Add to `config/default.toml`:

```toml
[agent_loop]
# Tool budget per turn
max_tool_calls = 10

# Reflection interval (every N tool calls)
reflection_interval = 5

# Progress check interval (every N tool calls)
progress_check_interval = 3

# Circuit breaker settings
[circuit_breaker]
max_repeats = 3
window_size = 10

# Context management
[context]
max_working_memory = 20
auto_summarize_interval = 10
summary_max_tokens = 500

# Timeouts
[timeouts]
tool_call_timeout_secs = 30
turn_timeout_secs = 300
```

### Validation Plan

1. **Unit Tests:**
   - GoalTracker: goal setting, progress tracking, completion detection
   - ToolBudget: budget enforcement, history tracking
   - ReflectionEngine: reflection triggering, summary generation
   - CircuitBreaker: loop detection, repeat detection
   - ContextManager: summarization, recovery

2. **Integration Tests:**
   - Complete agent loop with all components
   - Context recovery after restart
   - Budget exhaustion handling
   - Circuit breaker triggering

3. **Manual Testing:**
   - Long conversation (20+ turns)
   - Terminate and restart
   - Tool-heavy tasks
   - Edge cases (empty responses, timeouts)

### Success Criteria

1. **No Infinite Loops** — Agent never gets stuck in infinite tool call loops
2. **Fast Context Recovery** — Restart recovers context in < 5 seconds
3. **Budget Compliance** — Agent respects tool call budget 100% of the time
4. **Smart Reflection** — Agent reflects on progress and adjusts strategy
5. **Graceful Degradation** — Budget exhaustion produces useful partial results

## Implementation Order

### Phase 1: Safety Mechanisms (Fix Immediate Issues)
- Implement ToolBudget
- Implement CircuitBreaker
- Integrate into existing ReAct loop
- Add budget enforcement to tool_exec.rs

### Phase 2: Intelligence Layer (Improve Agent Quality)
- Implement GoalTracker
- Implement ReflectionEngine
- Add progress checks to step.rs
- Add reflection checkpoints

### Phase 3: Context Management (Fix Recovery Issues)
- Implement ContextManager
- Add auto-summarization
- Improve context recovery in chat.rs
- Add context compression

### Phase 4: Integration and Testing
- Integrate all components
- Add configuration support
- Comprehensive testing
- Performance optimization

## Files to Create

| File | Purpose |
|------|---------|
| `crates/runtime/src/core/react_loop/goal_tracker.rs` | GoalTracker implementation |
| `crates/runtime/src/core/react_loop/tool_budget.rs` | ToolBudget implementation |
| `crates/runtime/src/core/react_loop/reflection.rs` | ReflectionEngine implementation |
| `crates/runtime/src/core/react_loop/circuit_breaker.rs` | CircuitBreaker implementation |
| `crates/runtime/src/core/context_manager.rs` | ContextManager implementation |

## Risk Mitigation

1. **Performance Impact** — New components add overhead; mitigate with efficient data structures
2. **Complexity** — More components = more integration points; mitigate with clear interfaces
3. **Backward Compatibility** — Existing behavior must be preserved; mitigate with feature flags
4. **Testing Coverage** — Complex interactions; mitigate with comprehensive unit and integration tests

## References

- Current ReAct loop: `crates/runtime/src/core/react_loop/mod.rs`
- Tool execution: `crates/runtime/src/core/react_loop/tool_exec.rs`
- Chat handler: `crates/runtime/src/impl/daemon/handler/chat.rs`
- Storm Breaker: `crates/corpus/src/tools/tools/bash_exec.rs`
- Memory system: `crates/memory/src/`
