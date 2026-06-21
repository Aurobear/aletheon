# Agent Loop Redesign Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix cycle hanging, improve context recovery, and add tool execution safety to the Agent loop

**Architecture:** Add 5 new components (GoalTracker, ToolBudget, ReflectionEngine, CircuitBreaker, ContextManager) to the existing ReAct loop, with budget enforcement and loop detection

**Tech Stack:** Rust, tokio, existing base/cognit/corpus/memory crates

---

## File Structure

### New Files
- `crates/runtime/src/core/react_loop/tool_budget.rs` — Tool call budget management
- `crates/runtime/src/core/react_loop/circuit_breaker.rs` — Loop detection and forced stop
- `crates/runtime/src/core/react_loop/goal_tracker.rs` — Goal and sub-goal tracking
- `crates/runtime/src/core/react_loop/reflection.rs` — Periodic reflection engine
- `crates/runtime/src/core/context_manager.rs` — Layered context management

### Modified Files
- `crates/runtime/src/core/react_loop/mod.rs` — Integrate new components (fields, constructor, reset)
- `crates/runtime/src/core/react_loop/tool_exec.rs` — Add budget enforcement (streaming variant)
- `crates/runtime/src/core/react_loop/step.rs` — Add budget enforcement (non-streaming variant)
- `crates/runtime/src/impl/daemon/handler/chat.rs` — Integrate ContextManager
- `crates/runtime/src/core/event_sink.rs` — Add new event types
- `crates/runtime/src/core/config/agent.rs` — Add AgentLoopConfig, CircuitBreakerConfig structs
- `crates/runtime/src/core/config/mod.rs` — Re-export new config structs
- `config/default.toml` — Add default configuration under [agent] section

---

## Task 1: Implement ToolBudget (Safety Mechanism)

**Files:**
- Create: `crates/runtime/src/core/react_loop/tool_budget.rs`
- Modify: `crates/runtime/src/core/react_loop/mod.rs:1-2`

- [ ] **Step 1: Create ToolBudget module**

```rust
// crates/runtime/src/core/react_loop/tool_budget.rs
use std::time::Instant;
use tracing::warn;

/// Record of a single tool call for budget tracking.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub timestamp: Instant,
    pub success: bool,
}

/// Manages tool call budget per turn to prevent infinite loops.
#[derive(Debug)]
pub struct ToolBudget {
    max_calls: usize,
    used_calls: usize,
    call_history: Vec<ToolCallRecord>,
}

impl ToolBudget {
    /// Create a new budget with the given maximum calls per turn.
    pub fn new(max_calls: usize) -> Self {
        Self {
            max_calls,
            used_calls: 0,
            call_history: Vec::new(),
        }
    }

    /// Check if we can still make tool calls.
    pub fn can_call(&self) -> bool {
        self.used_calls < self.max_calls
    }

    /// Record a tool call and check budget.
    /// Returns true if the call was within budget, false if budget exceeded.
    pub fn record_call(&mut self, record: ToolCallRecord) -> bool {
        if !self.can_call() {
            warn!(
                tool = %record.tool_name,
                used = self.used_calls,
                max = self.max_calls,
                "Tool budget exceeded!"
            );
            return false;
        }

        self.used_calls += 1;
        self.call_history.push(record);
        true
    }

    /// Get remaining calls in budget.
    pub fn remaining(&self) -> usize {
        self.max_calls.saturating_sub(self.used_calls)
    }

    /// Check if budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.used_calls >= self.max_calls
    }

    /// Get total calls made.
    pub fn total_calls(&self) -> usize {
        self.used_calls
    }

    /// Get call history.
    pub fn history(&self) -> &[ToolCallRecord] {
        &self.call_history
    }

    /// Reset budget for a new turn.
    pub fn reset(&mut self) {
        self.used_calls = 0;
        self.call_history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_enforcement() {
        let mut budget = ToolBudget::new(3);

        assert!(budget.can_call());
        assert_eq!(budget.remaining(), 3);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: Instant::now(),
            success: true,
        });
        assert_eq!(budget.remaining(), 2);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: Instant::now(),
            success: true,
        });
        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: Instant::now(),
            success: true,
        });

        assert!(budget.is_exhausted());
        assert!(!budget.can_call());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_reset() {
        let mut budget = ToolBudget::new(2);

        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: Instant::now(),
            success: true,
        });
        budget.record_call(ToolCallRecord {
            tool_name: "test".into(),
            timestamp: Instant::now(),
            success: true,
        });

        assert!(budget.is_exhausted());

        budget.reset();
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining(), 2);
    }
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/runtime/src/core/react_loop/mod.rs` after line 2:

```rust
pub mod tool_budget;
```

- [ ] **Step 3: Run tests to verify**

```bash
cargo test -p runtime tool_budget
```

Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/react_loop/tool_budget.rs crates/runtime/src/core/react_loop/mod.rs
git commit -m "feat(runtime): add ToolBudget for tool call budget enforcement"
```

---

## Task 2: Implement CircuitBreaker (Loop Detection)

**Files:**
- Create: `crates/runtime/src/core/react_loop/circuit_breaker.rs`
- Modify: `crates/runtime/src/core/react_loop/mod.rs`

- [ ] **Step 1: Create CircuitBreaker module**

```rust
// crates/runtime/src/core/react_loop/circuit_breaker.rs
use std::collections::VecDeque;
use std::time::Instant;
use tracing::warn;

/// Signature of a tool call for loop detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallSignature {
    pub tool_name: String,
    pub args_hash: u64,
}

impl ToolCallSignature {
    pub fn new(tool_name: &str, args: &serde_json::Value) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        args.to_string().hash(&mut hasher);
        Self {
            tool_name: tool_name.to_string(),
            args_hash: hasher.finish(),
        }
    }
}

/// Status returned by circuit breaker check.
#[derive(Debug, Clone)]
pub enum CircuitBreakerStatus {
    /// No issues detected.
    Ok,
    /// Warning: pattern detected but not yet tripped.
    Warning(String),
    /// Circuit tripped: loop detected, must stop.
    Tripped(String),
}

/// Detects infinite loops and repeated tool calls.
#[derive(Debug)]
pub struct CircuitBreaker {
    recent_calls: VecDeque<ToolCallSignature>,
    max_repeats: usize,
    window_size: usize,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    /// - max_repeats: how many identical calls before tripping
    /// - window_size: how many recent calls to track
    pub fn new(max_repeats: usize, window_size: usize) -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(window_size),
            max_repeats,
            window_size,
        }
    }

    /// Check if a new tool call would trip the circuit breaker.
    /// Returns status indicating if it's safe to proceed.
    pub fn check(&mut self, call: &ToolCallSignature) -> CircuitBreakerStatus {
        // Count how many times this exact call appears in the window
        let count = self.recent_calls.iter().filter(|c| *c == call).count();

        if count >= self.max_repeats {
            let reason = format!(
                "Loop detected: tool '{}' with same args called {} times in last {} calls",
                call.tool_name, count + 1, self.window_size
            );
            warn!("{}", reason);
            CircuitBreakerStatus::Tripped(reason)
        } else if count >= self.max_repeats - 1 {
            let reason = format!(
                "Warning: tool '{}' with same args called {} times, will trip at {}",
                call.tool_name, count + 1, self.max_repeats
            );
            CircuitBreakerStatus::Warning(reason)
        } else {
            // Add to window
            if self.recent_calls.len() >= self.window_size {
                self.recent_calls.pop_front();
            }
            self.recent_calls.push_back(call.clone());
            CircuitBreakerStatus::Ok
        }
    }

    /// Reset the circuit breaker for a new turn.
    pub fn reset(&mut self) {
        self.recent_calls.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_loop() {
        let mut cb = CircuitBreaker::new(3, 10);
        let call1 = ToolCallSignature::new("read_file", &serde_json::json!({"path": "/a"}));
        let call2 = ToolCallSignature::new("read_file", &serde_json::json!({"path": "/b"}));

        assert!(matches!(cb.check(&call1), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call2), CircuitBreakerStatus::Ok));
    }

    #[test]
    fn test_loop_detection() {
        let mut cb = CircuitBreaker::new(3, 10);
        let call = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "ls"}));

        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Warning(_)));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Tripped(_)));
    }

    #[test]
    fn test_different_args_no_loop() {
        let mut cb = CircuitBreaker::new(3, 10);
        let call1 = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "ls"}));
        let call2 = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "pwd"}));

        for _ in 0..5 {
            assert!(matches!(cb.check(&call1), CircuitBreakerStatus::Ok));
            assert!(matches!(cb.check(&call2), CircuitBreakerStatus::Ok));
        }
    }

    #[test]
    fn test_reset() {
        let mut cb = CircuitBreaker::new(2, 10);
        let call = ToolCallSignature::new("test", &serde_json::json!({}));

        cb.check(&call);
        cb.check(&call);
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Tripped(_)));

        cb.reset();
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
    }
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/runtime/src/core/react_loop/mod.rs` after tool_budget:

```rust
pub mod circuit_breaker;
```

- [ ] **Step 3: Run tests to verify**

```bash
cargo test -p runtime circuit_breaker
```

Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/react_loop/circuit_breaker.rs crates/runtime/src/core/react_loop/mod.rs
git commit -m "feat(runtime): add CircuitBreaker for loop detection"
```

---

## Task 3: Implement GoalTracker (Intelligence Layer)

**Files:**
- Create: `crates/runtime/src/core/react_loop/goal_tracker.rs`
- Modify: `crates/runtime/src/core/react_loop/mod.rs`

- [ ] **Step 1: Create GoalTracker module**

```rust
// crates/runtime/src/core/react_loop/goal_tracker.rs
use std::time::Instant;
use tracing::info;

/// Status of a goal.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
}

/// A single goal with metadata.
#[derive(Debug, Clone)]
pub struct Goal {
    pub description: String,
    pub created_at: Instant,
    pub status: GoalStatus,
}

/// A sub-goal under the main goal.
#[derive(Debug, Clone)]
pub struct SubGoal {
    pub description: String,
    pub completed: bool,
}

/// Tracks the current goal and sub-goals for the agent.
#[derive(Debug)]
pub struct GoalTracker {
    current_goal: Option<Goal>,
    sub_goals: Vec<SubGoal>,
    success_criteria: Vec<String>,
}

impl GoalTracker {
    /// Create a new empty goal tracker.
    pub fn new() -> Self {
        Self {
            current_goal: None,
            sub_goals: Vec::new(),
            success_criteria: Vec::new(),
        }
    }

    /// Set the main goal for this turn.
    pub fn set_goal(&mut self, goal: String) {
        info!(goal = %goal, "Setting agent goal");
        self.current_goal = Some(Goal {
            description: goal,
            created_at: Instant::now(),
            status: GoalStatus::InProgress,
        });
    }

    /// Add a sub-goal.
    pub fn add_sub_goal(&mut self, sub_goal: String) {
        if self.sub_goals.len() < 3 {
            self.sub_goals.push(SubGoal {
                description: sub_goal,
                completed: false,
            });
        }
    }

    /// Add a success criterion.
    pub fn add_success_criterion(&mut self, criterion: String) {
        self.success_criteria.push(criterion);
    }

    /// Mark a sub-goal as completed.
    pub fn complete_sub_goal(&mut self, index: usize) {
        if let Some(sg) = self.sub_goals.get_mut(index) {
            sg.completed = true;
            info!(sub_goal = %sg.description, "Sub-goal completed");
        }
    }

    /// Mark the main goal as completed.
    pub fn complete_goal(&mut self) {
        if let Some(ref mut goal) = self.current_goal {
            goal.status = GoalStatus::Completed;
            info!(goal = %goal.description, "Goal completed");
        }
    }

    /// Mark the main goal as failed.
    pub fn fail_goal(&mut self, reason: &str) {
        if let Some(ref mut goal) = self.current_goal {
            goal.status = GoalStatus::Failed;
            info!(goal = %goal.description, reason = %reason, "Goal failed");
        }
    }

    /// Check if the goal is complete.
    pub fn is_complete(&self) -> bool {
        self.current_goal
            .as_ref()
            .map(|g| g.status == GoalStatus::Completed)
            .unwrap_or(false)
    }

    /// Check if all sub-goals are complete.
    pub fn all_sub_goals_complete(&self) -> bool {
        !self.sub_goals.is_empty() && self.sub_goals.iter().all(|sg| sg.completed)
    }

    /// Get context string for LLM reasoning.
    pub fn get_context(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref goal) = self.current_goal {
            parts.push(format!("Current goal: {}", goal.description));
        }

        if !self.sub_goals.is_empty() {
            let sub_goal_strs: Vec<String> = self
                .sub_goals
                .iter()
                .enumerate()
                .map(|(i, sg)| {
                    let status = if sg.completed { "✓" } else { "○" };
                    format!("  {}{}. {}", status, i + 1, sg.description)
                })
                .collect();
            parts.push(format!("Sub-goals:\n{}", sub_goal_strs.join("\n")));
        }

        if !self.success_criteria.is_empty() {
            parts.push(format!(
                "Success criteria: {}",
                self.success_criteria.join(", ")
            ));
        }

        parts.join("\n")
    }

    /// Reset for a new turn.
    pub fn reset(&mut self) {
        self.current_goal = None;
        self.sub_goals.clear();
        self.success_criteria.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_setting() {
        let mut tracker = GoalTracker::new();
        assert!(!tracker.is_complete());

        tracker.set_goal("Create a hello world program".into());
        assert!(!tracker.is_complete());

        tracker.complete_goal();
        assert!(tracker.is_complete());
    }

    #[test]
    fn test_sub_goals() {
        let mut tracker = GoalTracker::new();
        tracker.set_goal("Build a website".into());
        tracker.add_sub_goal("Create HTML file".into());
        tracker.add_sub_goal("Add CSS styling".into());
        tracker.add_sub_goal("Add JavaScript".into());

        assert!(!tracker.all_sub_goals_complete());

        tracker.complete_sub_goal(0);
        tracker.complete_sub_goal(1);
        tracker.complete_sub_goal(2);

        assert!(tracker.all_sub_goals_complete());
    }

    #[test]
    fn test_max_sub_goals() {
        let mut tracker = GoalTracker::new();
        tracker.add_sub_goal("1".into());
        tracker.add_sub_goal("2".into());
        tracker.add_sub_goal("3".into());
        tracker.add_sub_goal("4".into()); // Should be ignored

        assert_eq!(tracker.sub_goals.len(), 3);
    }

    #[test]
    fn test_context_generation() {
        let mut tracker = GoalTracker::new();
        tracker.set_goal("Write tests".into());
        tracker.add_sub_goal("Unit tests".into());
        tracker.add_success_criterion("All tests pass".into());

        let ctx = tracker.get_context();
        assert!(ctx.contains("Write tests"));
        assert!(ctx.contains("Unit tests"));
        assert!(ctx.contains("All tests pass"));
    }

    #[test]
    fn test_reset() {
        let mut tracker = GoalTracker::new();
        tracker.set_goal("test".into());
        tracker.add_sub_goal("sub".into());

        tracker.reset();
        assert!(!tracker.is_complete());
        assert!(tracker.sub_goals.is_empty());
    }
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/runtime/src/core/react_loop/mod.rs` after circuit_breaker:

```rust
pub mod goal_tracker;
```

- [ ] **Step 3: Run tests to verify**

```bash
cargo test -p runtime goal_tracker
```

Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/react_loop/goal_tracker.rs crates/runtime/src/core/react_loop/mod.rs
git commit -m "feat(runtime): add GoalTracker for goal and sub-goal tracking"
```

---

## Task 4: Implement ReflectionEngine (Intelligence Layer)

**Files:**
- Create: `crates/runtime/src/core/react_loop/reflection.rs`
- Modify: `crates/runtime/src/core/react_loop/mod.rs`

- [ ] **Step 1: Create ReflectionEngine module**

```rust
// crates/runtime/src/core/react_loop/reflection.rs
use tracing::info;

/// Recommendation from reflection.
#[derive(Debug, Clone)]
pub enum ReflectionRecommendation {
    /// Continue with current strategy.
    Continue,
    /// Adjust strategy with suggestion.
    AdjustStrategy(String),
    /// Stop with reason.
    Stop(TerminationReason),
}

/// Reason for stopping (named differently to avoid collision with cognit::StopReason).
#[derive(Debug, Clone)]
pub enum TerminationReason {
    /// Goal achieved.
    GoalAchieved,
    /// Stuck in a loop.
    StuckInLoop,
    /// Budget exhausted.
    BudgetExhausted,
    /// Timeout.
    Timeout,
    /// User request.
    UserRequest,
}

/// Context provided to the reflection engine.
#[derive(Debug, Clone)]
pub struct ReflectionContext {
    pub goal: Option<String>,
    pub recent_actions: Vec<String>,
    pub current_state: String,
    pub tool_calls_made: usize,
    pub errors: usize,
}

/// Result of a reflection.
#[derive(Debug, Clone)]
pub struct ReflectionResult {
    pub summary: String,
    pub recommendation: ReflectionRecommendation,
}

/// Periodic reflection engine for the agent loop.
#[derive(Debug)]
pub struct ReflectionEngine {
    reflection_interval: usize,
    calls_since_reflection: usize,
}

impl ReflectionEngine {
    /// Create a new reflection engine.
    /// - reflection_interval: reflect every N tool calls
    pub fn new(reflection_interval: usize) -> Self {
        Self {
            reflection_interval,
            calls_since_reflection: 0,
        }
    }

    /// Check if it's time to reflect.
    pub fn should_reflect(&self) -> bool {
        self.calls_since_reflection >= self.reflection_interval
    }

    /// Record a tool call and check if reflection is needed.
    pub fn record_call(&mut self) -> bool {
        self.calls_since_reflection += 1;
        self.should_reflect()
    }

    /// Perform reflection and return recommendation.
    pub fn reflect(&mut self, context: &ReflectionContext) -> ReflectionResult {
        info!(
            tool_calls = context.tool_calls_made,
            errors = context.errors,
            "Performing reflection"
        );

        self.calls_since_reflection = 0;

        // Analyze the situation
        let error_rate = if context.tool_calls_made > 0 {
            context.errors as f64 / context.tool_calls_made as f64
        } else {
            0.0
        };

        let recommendation = if error_rate > 0.5 {
            ReflectionRecommendation::Stop(StopReason::StuckInLoop)
        } else if context.tool_calls_made >= 10 {
            ReflectionRecommendation::Stop(StopReason::BudgetExhausted)
        } else {
            ReflectionRecommendation::Continue
        };

        let summary = format!(
            "Reflection: {} tool calls made, {} errors ({:.0}% error rate). {}",
            context.tool_calls_made,
            context.errors,
            error_rate * 100.0,
            match &recommendation {
                ReflectionRecommendation::Continue => "Continuing...",
                ReflectionRecommendation::AdjustStrategy(s) => {
                    &format!("Adjusting: {}", s)
                }
                ReflectionRecommendation::Stop(reason) => {
                    &format!("Stopping: {:?}", reason)
                }
            }
        );

        ReflectionResult {
            summary,
            recommendation,
        }
    }

    /// Reset for a new turn.
    pub fn reset(&mut self) {
        self.calls_since_reflection = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflection_interval() {
        let mut engine = ReflectionEngine::new(3);

        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(engine.should_reflect());
    }

    #[test]
    fn test_reflection_resets_counter() {
        let mut engine = ReflectionEngine::new(2);

        engine.record_call();
        engine.record_call();
        assert!(engine.should_reflect());

        let ctx = ReflectionContext {
            goal: Some("test".into()),
            recent_actions: vec![],
            current_state: "ok".into(),
            tool_calls_made: 2,
            errors: 0,
        };
        engine.reflect(&ctx);

        assert!(!engine.should_reflect());
        assert_eq!(engine.calls_since_reflection, 0);
    }

    #[test]
    fn test_high_error_rate_stops() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = ReflectionContext {
            goal: Some("test".into()),
            recent_actions: vec![],
            current_state: "error".into(),
            tool_calls_made: 10,
            errors: 6,
        };

        let result = engine.reflect(&ctx);
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::Stop(StopReason::StuckInLoop)
        ));
    }
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/runtime/src/core/react_loop/mod.rs` after goal_tracker:

```rust
pub mod reflection;
```

- [ ] **Step 3: Run tests to verify**

```bash
cargo test -p runtime reflection
```

Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/react_loop/reflection.rs crates/runtime/src/core/react_loop/mod.rs
git commit -m "feat(runtime): add ReflectionEngine for periodic reflection"
```

---

## Task 5: Add Configuration Support

**Files:**
- Modify: `crates/runtime/src/core/config/agent.rs` — Add config structs and fields to RuntimeConfig
- Modify: `crates/runtime/src/core/config/mod.rs` — Re-export new config structs
- Modify: `config/default.toml` — Add default configuration

- [ ] **Step 1: Add config structs to agent.rs**

Add to `crates/runtime/src/core/config/agent.rs` after the existing config structs (around line 153):

```rust
/// Agent loop configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopConfig {
    /// Maximum tool calls per turn.
    pub max_tool_calls: usize,
    /// Reflection interval (every N tool calls).
    pub reflection_interval: usize,
    /// Progress check interval (every N tool calls).
    pub progress_check_interval: usize,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: 10,
            reflection_interval: 5,
            progress_check_interval: 3,
        }
    }
}

/// Circuit breaker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Maximum repeated calls before tripping.
    pub max_repeats: usize,
    /// Window size for tracking recent calls.
    pub window_size: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            max_repeats: 3,
            window_size: 10,
        }
    }
}
```

- [ ] **Step 2: Add fields to RuntimeConfig**

In the `RuntimeConfig` struct, add new fields with `#[serde(default)]`:

```rust
pub struct RuntimeConfig {
    // ... existing fields ...
    #[serde(default)]
    pub agent_loop: AgentLoopConfig,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
}
```

Update `Default` impl to include the new fields.

- [ ] **Step 3: Add re-exports to mod.rs**

Add to `crates/runtime/src/core/config/mod.rs`:

```rust
pub use agent::{AgentLoopConfig, CircuitBreakerConfig};
```

- [ ] **Step 4: Add to default.toml**

Add to `config/default.toml` under the `[agent]` section (or as top-level if that's the pattern):

```toml
[agent.agent_loop]
# Maximum tool calls per turn
max_tool_calls = 10

# Reflection interval (every N tool calls)
reflection_interval = 5

# Progress check interval (every N tool calls)
progress_check_interval = 3

[agent.circuit_breaker]
# Maximum repeated calls before tripping
max_repeats = 3

# Window size for tracking recent calls
window_size = 10
```

- [ ] **Step 3: Verify configuration loads**

```bash
cargo build -p runtime
```

Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/config/mod.rs config/default.toml
git commit -m "feat(runtime): add agent_loop and circuit_breaker configuration"
```

---

## Task 6: Integrate Safety Mechanisms into ReAct Loop

**Files:**
- Modify: `crates/runtime/src/core/react_loop/mod.rs` — Add fields, constructor, reset
- Modify: `crates/runtime/src/core/react_loop/tool_exec.rs` — Add budget/circuit-breaker to streaming variant
- Modify: `crates/runtime/src/core/react_loop/step.rs` — Add budget/circuit-breaker to non-streaming variant

- [ ] **Step 1: Add new components to ReActLoop struct**

In `crates/runtime/src/core/react_loop/mod.rs`, add imports and fields:

```rust
use tool_budget::ToolBudget;
use circuit_breaker::{CircuitBreaker, CircuitBreakerStatus, ToolCallSignature};
use goal_tracker::GoalTracker;
use reflection::ReflectionEngine;

// Add to ReActLoop struct:
pub struct ReActLoop {
    // ... existing fields ...
    tool_budget: ToolBudget,
    circuit_breaker: CircuitBreaker,
    goal_tracker: GoalTracker,
    reflection_engine: ReflectionEngine,
}
```

- [ ] **Step 2: Initialize new components in constructor**

Update `ReActLoop::new()`:

```rust
pub fn new(config: RuntimeConfig) -> Self {
    // ... existing code ...
    Self {
        // ... existing fields ...
        tool_budget: ToolBudget::new(config.agent_loop.max_tool_calls),
        circuit_breaker: CircuitBreaker::new(
            config.circuit_breaker.max_repeats,
            config.circuit_breaker.window_size,
        ),
        goal_tracker: GoalTracker::new(),
        reflection_engine: ReflectionEngine::new(config.agent_loop.reflection_interval),
    }
}
```

- [ ] **Step 3: Add budget check to tool execution**

In `crates/runtime/src/core/react_loop/tool_exec.rs`, add budget check before tool execution:

```rust
// Before executing each tool call, check budget:
for (id, name, input) in &tool_calls {
    // Check tool budget
    if !self.tool_budget.can_call() {
        warn!("Tool budget exhausted, stopping loop");
        let msg = format!(
            "Tool budget exhausted after {} calls. Partial result: {}",
            self.tool_budget.total_calls(),
            text_parts.join(" ")
        );
        event_sink.emit(Event::TurnDone { result: Ok(msg.clone()) });
        return Ok((msg, TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        }));
    }

    // Check circuit breaker
    let signature = ToolCallSignature::new(name, input);
    match self.circuit_breaker.check(&signature) {
        CircuitBreakerStatus::Tripped(reason) => {
            warn!("Circuit breaker tripped: {}", reason);
            let msg = format!("Loop detected: {}. Stopping.", reason);
            event_sink.emit(Event::TurnDone { result: Ok(msg.clone()) });
            return Ok((msg, TurnMetrics {
                tool_calls_made,
                tool_errors,
                elapsed_ms: start.elapsed().as_millis() as u64,
                iterations: self.iteration,
                completed_normally: false,
            }));
        }
        CircuitBreakerStatus::Warning(reason) => {
            warn!("Circuit breaker warning: {}", reason);
        }
        CircuitBreakerStatus::Ok => {}
    }

    // ... existing tool execution code ...

    // Record call in budget
    self.tool_budget.record_call(tool_budget::ToolCallRecord {
        tool_name: name.clone(),
        timestamp: std::time::Instant::now(),
        success: !is_error,
    });
}
```

- [ ] **Step 4: Add reset for new components**

Update `ReActLoop::reset()`:

```rust
pub fn reset(&mut self) {
    // ... existing reset code ...
    self.tool_budget.reset();
    self.circuit_breaker.reset();
    self.goal_tracker.reset();
    self.reflection_engine.reset();
}
```

- [ ] **Step 5: Run existing tests to verify no regressions**

```bash
cargo test -p runtime
```

Expected: All existing tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/runtime/src/core/react_loop/mod.rs crates/runtime/src/core/react_loop/tool_exec.rs crates/runtime/src/core/react_loop/step.rs
git commit -m "feat(runtime): integrate ToolBudget and CircuitBreaker into ReAct loop"
```

---

## Task 7: Add New Event Types

**Files:**
- Modify: `crates/runtime/src/core/event_sink.rs`

- [ ] **Step 1: Add new event variants**

```rust
pub enum Event {
    // ... existing events ...

    /// Agent goal was set.
    GoalSet {
        goal: String,
        sub_goals: Vec<String>,
    },

    /// Reflection completed.
    Reflection {
        summary: String,
        recommendation: String,
    },

    /// Tool budget exceeded.
    BudgetExceeded {
        used: usize,
        max: usize,
    },

    /// Circuit breaker tripped.
    CircuitBreakerTripped {
        reason: String,
    },
}
```

- [ ] **Step 2: Update event_to_json in format.rs**

Add JSON serialization for new events:

```rust
Event::GoalSet { goal, sub_goals } => {
    json!({
        "type": "goal_set",
        "goal": goal,
        "sub_goals": sub_goals,
    })
}
Event::Reflection { summary, recommendation } => {
    json!({
        "type": "reflection",
        "summary": summary,
        "recommendation": recommendation,
    })
}
Event::BudgetExceeded { used, max } => {
    json!({
        "type": "budget_exceeded",
        "used": used,
        "max": max,
    })
}
Event::CircuitBreakerTripped { reason } => {
    json!({
        "type": "circuit_breaker_tripped",
        "reason": reason,
    })
}
```

- [ ] **Step 3: Run tests to verify**

```bash
cargo test -p runtime
```

Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/event_sink.rs crates/runtime/src/impl/daemon/handler/format.rs
git commit -m "feat(runtime): add GoalSet, Reflection, BudgetExceeded, CircuitBreakerTripped events"
```

---

## Task 8: Integration Testing

**Files:**
- Modify: `crates/runtime/src/core/react_loop/mod.rs` (add integration tests)

- [ ] **Step 1: Add integration test for budget enforcement**

```rust
#[tokio::test]
async fn test_budget_enforcement_stops_loop() {
    let cfg = RuntimeConfig {
        max_iterations: 100,
        session_id: "test".into(),
        learning_enabled: false,
        compaction_enabled: false,
        agent_loop: AgentLoopConfig {
            max_tool_calls: 3,
            ..Default::default()
        },
        ..RuntimeConfig::default()
    };

    let mut lp = ReActLoop::new(cfg);
    let llm = BigToolLlm {
        calls: Mutex::new(0),
        tool_until: 100, // Would run forever without budget
    };
    let tool_defs: Vec<ToolDefinition> = vec![];

    let (out, metrics) = lp
        .run(
            "do many things",
            &llm,
            &tool_defs,
            |_id: &str, name: &str, _input: &serde_json::Value| {
                let name = name.to_string();
                async move { (format!("result_{}", name), false) }
            },
        )
        .await
        .unwrap();

    // Should stop after 3 tool calls due to budget
    assert!(metrics.tool_calls_made <= 3);
    assert!(!metrics.completed_normally);
}
```

- [ ] **Step 2: Run integration tests**

```bash
cargo test -p runtime --test '*'
```

Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/runtime/src/core/react_loop/mod.rs
git commit -m "test(runtime): add integration tests for budget enforcement"
```

---

## Task 9: Manual Testing

- [ ] **Step 1: Build the project**

```bash
cargo build --release
```

Expected: Build succeeds

- [ ] **Step 2: Test basic conversation**

```bash
target/release/aletheon-exec --prompt "say hello"
```

Expected: Normal response

- [ ] **Step 3: Test tool execution**

```bash
target/release/aletheon-exec --prompt "list files in current directory"
```

Expected: Lists files successfully

- [ ] **Step 4: Test long conversation (verify no hanging)**

```bash
tmux new-session -d -s test
tmux send-keys -t test "target/release/aletheon" Enter
sleep 2
tmux send-keys -t test "create a rust program that calculates fibonacci" Enter
sleep 15
tmux capture-pane -t test -p
```

Expected: Agent creates the program without getting stuck

- [ ] **Step 5: Test termination and restart**

```bash
tmux send-keys -t test C-c C-c
sleep 1
tmux send-keys -t test "target/release/aletheon" Enter
sleep 2
tmux send-keys -t test "what did we just do?" Enter
sleep 5
tmux capture-pane -t test -p
```

Expected: Agent responds without getting stuck in a loop

- [ ] **Step 6: Clean up**

```bash
tmux kill-session -t test
```

---

## Summary

This plan implements the Agent Loop Redesign in 9 tasks:

1. **ToolBudget** — Prevents infinite tool call loops
2. **CircuitBreaker** — Detects and stops repeated patterns
3. **GoalTracker** — Tracks goals and sub-goals
4. **ReflectionEngine** — Periodic reflection on progress
5. **Configuration** — Adds agent_loop and circuit_breaker config
6. **Integration** — Integrates safety mechanisms into ReAct loop
7. **Events** — Adds new event types for TUI
8. **Integration Tests** — Verifies budget enforcement works
9. **Manual Testing** — End-to-end verification

**Expected Outcome:**
- No more infinite loops
- Tool budget enforced (max 10 calls per turn)
- Circuit breaker detects repeated patterns
- Agent reflects on progress every 5 tool calls
- Graceful degradation when budget exhausted
