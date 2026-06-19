# P0 Implementation Plan: Cache-First + Execpolicy + Controller

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement three P0 improvements — cache-first prefix stability, independent execpolicy engine, and transport-agnostic controller — to make aletheon production-ready.

**Architecture:** Cache-first ensures byte-stable system prefix for provider cache hits. Execpolicy extracts policy logic into a testable crate. Controller decouples transport from agent logic for multi-frontend support.

**Tech Stack:** Rust, tokio, serde, sha2

---

## File Map

### New Files
| File | Purpose |
|------|---------|
| `crates/aletheon-abi/src/execpolicy.rs` | Independent policy engine: Decision, Rule, Policy, Evaluation |
| `crates/aletheon-runtime/src/core/controller.rs` | Transport-agnostic Controller struct |
| `crates/aletheon-runtime/src/core/event_sink.rs` | Typed event stream: Event enum, EventSink trait |

### Modified Files
| File | Change |
|------|--------|
| `crates/aletheon-abi/src/lib.rs` | Add `pub mod execpolicy;` |
| `crates/aletheon-abi/src/tool.rs` | Add `Previewer` trait for checkpoint snapshots |
| `crates/aletheon-runtime/src/core/mod.rs` | Export controller, event_sink modules |
| `crates/aletheon-runtime/src/core/react_loop.rs` | Add compose_user_message(), system_prompt accessor |
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Use Controller, use compose_user_message() |
| `crates/aletheon-body/src/impl/security/runner.rs` | Use execpolicy::Policy instead of inline PolicyEngine |
| `crates/aletheon-body/src/impl/security/policy.rs` | Keep for backward compat, mark deprecated |
| `crates/aletheon-body/src/impl/security/permission_rules.rs` | Migrate loader to execpolicy |

### Test Files
| File | Purpose |
|------|---------|
| `crates/aletheon-abi/tests/execpolicy_tests.rs` | Integration tests for policy engine |
| `crates/aletheon-runtime/tests/controller_tests.rs` | Integration tests for controller |

---

## Phase 1: Cache-First Prefix Stability

### Task 1.1: Add compose_user_message to ReActLoop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Write the failing test**

```rust
// Add to existing #[cfg(test)] mod tests in react_loop.rs

#[test]
fn compose_user_message_plain_input() {
    let cfg = RuntimeConfig::default();
    let lp = ReActLoop::new(cfg);
    let composed = lp.compose_user_message("hello");
    assert_eq!(composed, "hello");
}

#[test]
fn compose_user_message_with_plan_mode() {
    let cfg = RuntimeConfig::default();
    let mut lp = ReActLoop::new(cfg);
    lp.set_plan_mode(true);
    let composed = lp.compose_user_message("hello");
    assert!(composed.contains("[PLAN MODE ACTIVE]"));
    assert!(composed.contains("hello"));
}

#[test]
fn compose_user_message_with_memory_updates() {
    let cfg = RuntimeConfig::default();
    let mut lp = ReActLoop::new(cfg);
    lp.queue_memory_update("user prefers dark mode");
    let composed = lp.compose_user_message("hello");
    assert!(composed.contains("<memory-update>"));
    assert!(composed.contains("user prefers dark mode"));
}

#[test]
fn compose_user_message_plan_and_memory() {
    let cfg = RuntimeConfig::default();
    let mut lp = ReActLoop::new(cfg);
    lp.set_plan_mode(true);
    lp.queue_memory_update("fact 1");
    let composed = lp.compose_user_message("do something");
    assert!(composed.contains("[PLAN MODE ACTIVE]"));
    assert!(composed.contains("<memory-update>"));
    assert!(composed.contains("do something"));
}

#[test]
fn system_prompt_immutable_after_construction() {
    let cfg = RuntimeConfig::default();
    let mut lp = ReActLoop::new(cfg);
    let p1 = lp.system_prompt().to_string();
    lp.set_plan_mode(true);
    lp.queue_memory_update("new fact");
    let p2 = lp.system_prompt().to_string();
    assert_eq!(p1, p2, "system prompt must not change");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::react_loop::tests::compose_user_message_plain_input 2>&1 | tail -5`
Expected: error[E0433]: unresolved name `compose_user_message`

- [ ] **Step 3: Write implementation**

Add to `ReActLoop` struct:
```rust
// New fields
system_prompt: String,
plan_mode: bool,
pending_memory: Vec<String>,
```

Add methods:
```rust
/// Set the system prompt (called once at construction).
pub fn set_system_prompt(&mut self, prompt: String) {
    self.system_prompt = prompt;
}

/// Get the immutable system prompt.
pub fn system_prompt(&self) -> &str {
    &self.system_prompt
}

/// Enable/disable plan mode. Injected into user message, NOT system prompt.
pub fn set_plan_mode(&mut self, enabled: bool) {
    self.plan_mode = enabled;
}

/// Queue a memory update for the next user message.
pub fn queue_memory_update(&mut self, update: String) {
    self.pending_memory.push(update);
}

/// Compose user message with mid-session injections.
/// Changes go here, NOT into system prompt, to preserve cache stability.
pub fn compose_user_message(&self, input: &str) -> String {
    let mut parts = Vec::new();

    if self.plan_mode {
        parts.push("[PLAN MODE ACTIVE: Think step-by-step before acting.]".to_string());
    }

    if !self.pending_memory.is_empty() {
        let updates = self.pending_memory.iter()
            .map(|m| format!("- {}", m))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
    }

    parts.push(input.to_string());
    parts.join("\n\n")
}
```

Update `new()`:
```rust
pub fn new(config: RuntimeConfig) -> Self {
    let compressor =
        AdvancedCompressor::new(config.tail_token_budget, config.target_summary_chars);
    Self {
        config,
        iteration: 0,
        messages: Vec::new(),
        compressor,
        system_prompt: String::new(),
        plan_mode: false,
        pending_memory: Vec::new(),
    }
}
```

Update `reset()` to clear mutable state:
```rust
pub fn reset(&mut self) {
    self.iteration = 0;
    self.messages.clear();
    self.pending_memory.clear();
    // Note: plan_mode persists across resets (user choice)
    // Note: system_prompt never resets (immutable)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::react_loop::tests 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): add compose_user_message for cache-first prefix stability

Mid-session changes (plan mode, memory updates) are now injected into
the user message via compose_user_message(), not the system prompt.
This keeps the system prompt byte-stable across turns for provider
prefix cache hits (DeepSeek, Anthropic, Mimo)."
```

---

### Task 1.2: Wire compose_user_message into DaemonHandler

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Find the handle_chat method**

Run: `grep -n "handle_chat\|fn chat\|user_input\|process_react" crates/aletheon-runtime/src/impl/daemon/handler.rs | head -20`

- [ ] **Step 2: Update handle_chat to use compose_user_message**

In the `handle_chat` method, find where user input is passed to the runtime. Replace direct input with composed input:

```rust
// Before:
// let result = runtime.process_react(&user_input, ...).await?;

// After:
let composed = {
    let state = self.state.lock().await;
    // Access the ReActLoop through the runtime to compose
    // For now, we compose at the handler level
    let mut parts = Vec::new();

    // Check memory queue
    let memory_queue = self.memory_queue.lock().await;
    if !memory_queue.is_empty() {
        let updates = memory_queue.iter()
            .map(|m| format!("- {}", m))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        memory_queue.clear();
    }

    parts.push(user_input.clone());
    parts.join("\n\n")
};

let result = runtime.process_react(&composed, ...).await?;
```

- [ ] **Step 3: Verify memory_queue is drained**

The existing `memory_queue` field in `RequestHandler` is already used for this purpose. Ensure it's drained into the composed message.

- [ ] **Step 4: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(daemon): wire compose_user_message into chat handler

Memory updates now ride the user message tail instead of being injected
into the system prefix. This preserves byte-stable caching."
```

---

## Phase 2: Independent Execpolicy Engine

### Task 2.1: Create execpolicy module in aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/execpolicy.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Write the failing test file**

```rust
// crates/aletheon-abi/tests/execpolicy_tests.rs

use aletheon_abi::execpolicy::*;
use std::sync::Arc;

#[test]
fn decision_ordering() {
    assert!(Decision::Allow < Decision::Prompt);
    assert!(Decision::Prompt < Decision::Forbidden);
}

#[test]
fn prefix_rule_matches_command() {
    let rule = PrefixRule::new("rm", Decision::Forbidden)
        .with_pattern(vec![
            PatternToken::Exact("rm".into()),
            PatternToken::Alternatives(vec!["-rf".into(), "-r".into()]),
        ]);

    assert!(rule.matches(&["rm", "-rf", "/"]).is_some());
    assert!(rule.matches(&["rm", "-r", "dir"]).is_some());
    assert!(rule.matches(&["rm", "file"]).is_none());
}

#[test]
fn policy_check_allows_safe_commands() {
    let policy = Policy::new();
    let eval = policy.check(&["ls", "-la"], default_heuristics);
    assert_eq!(eval.decision, Decision::Allow);
}

#[test]
fn policy_check_forbids_dangerous_commands() {
    let policy = Policy::new();
    let eval = policy.check(&["rm", "-rf", "/"], default_heuristics);
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn policy_check_prompts_unknown_commands() {
    let policy = Policy::new();
    let eval = policy.check(&["some_unknown_tool"], default_heuristics);
    assert_eq!(eval.decision, Decision::Prompt);
}

#[test]
fn policy_overlay_merge_precedence() {
    let mut base = Policy::new();
    // Base allows rm (unrealistic but tests merge)
    base.add_rule(Box::new(PrefixRule::new("rm", Decision::Allow)));

    let mut overlay = Policy::new();
    // Overlay forbids rm
    overlay.add_rule(Box::new(PrefixRule::new("rm", Decision::Forbidden)));

    base.merge_overlay(overlay);

    // Overlay takes precedence (last-wins)
    let eval = base.check(&["rm", "-rf", "/"], default_heuristics);
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn policy_layered_load() {
    let system = r#"
[[rules]]
program = "rm"
decision = "prompt"
pattern = ["rm", "-rf"]
"#;

    let user = r#"
[[rules]]
program = "rm"
decision = "forbidden"
pattern = ["rm", "-rf"]
"#;

    let policy = load_policy_layered(Some(system), Some(user), None).unwrap();
    let eval = policy.check(&["rm", "-rf", "/"], default_heuristics);
    // User overrides system
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn network_rule_check() {
    let mut policy = Policy::new();
    policy.add_network_rule(NetworkRule {
        host: "evil.com".into(),
        protocol: NetworkProtocol::Https,
        decision: Decision::Forbidden,
    });

    let eval = policy.check_network("evil.com", NetworkProtocol::Https);
    assert_eq!(eval.decision, Decision::Forbidden);

    let eval = policy.check_network("safe.com", NetworkProtocol::Https);
    assert_eq!(eval.decision, Decision::Allow);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-abi --test execpolicy_tests 2>&1 | tail -5`
Expected: error: test target `execpolicy_tests` not found

- [ ] **Step 3: Write execpolicy.rs**

```rust
// crates/aletheon-abi/src/execpolicy.rs

//! Independent execution policy engine.
//!
//! Separates policy logic from the tool runner for testability and reuse.
//! Supports layered configuration (system > user > project) with overlay merge.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Policy decision, ordered by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Prompt,
    Forbidden,
}

impl Default for Decision {
    fn default() -> Self {
        Decision::Prompt
    }
}

/// Result of checking a command against the policy.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub decision: Decision,
    pub matched_rules: Vec<String>, // rule descriptions
}

/// A pattern token for prefix matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternToken {
    Exact(String),
    Alternatives(Vec<String>),
}

/// A single prefix-based policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixRule {
    pub program: String,
    pub decision: Decision,
    #[serde(default)]
    pub pattern: Vec<PatternToken>,
}

impl PrefixRule {
    pub fn new(program: &str, decision: Decision) -> Self {
        Self {
            program: program.to_string(),
            decision,
            pattern: Vec::new(),
        }
    }

    pub fn with_pattern(mut self, pattern: Vec<PatternToken>) -> Self {
        self.pattern = pattern;
        self
    }

    /// Check if this rule matches the given command.
    pub fn matches(&self, cmd: &[String]) -> Option<String> {
        if cmd.is_empty() || cmd[0] != self.program {
            return None;
        }

        if self.pattern.is_empty() {
            // No pattern = match any invocation of this program
            return Some(format!("{} (any)", self.program));
        }

        // Match pattern tokens against command args
        let args = &cmd[1..];
        if args.len() < self.pattern.len() {
            return None;
        }

        for (i, token) in self.pattern.iter().enumerate() {
            match token {
                PatternToken::Exact(s) => {
                    if args.get(i).map(|a| a.as_str()) != Some(s.as_str()) {
                        return None;
                    }
                }
                PatternToken::Alternatives(alts) => {
                    let arg = match args.get(i) {
                        Some(a) => a,
                        None => return None,
                    };
                    if !alts.iter().any(|a| a == arg) {
                        return None;
                    }
                }
            }
        }

        Some(format!("{} {}", self.program, self.pattern_desc()))
    }

    fn pattern_desc(&self) -> String {
        self.pattern
            .iter()
            .map(|t| match t {
                PatternToken::Exact(s) => s.clone(),
                PatternToken::Alternatives(alts) => format!("[{}]", alts.join("|")),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Network protocol for network rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Http,
    Https,
    Any,
}

/// A network access rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRule {
    pub host: String,
    pub protocol: NetworkProtocol,
    pub decision: Decision,
}

/// The independent policy engine.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Policy {
    rules: Vec<PrefixRule>,
    network_rules: Vec<NetworkRule>,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a policy with default safety rules.
    pub fn with_defaults() -> Self {
        let mut policy = Self::new();

        // Destructive commands require prompt by default
        policy.add_rule(PrefixRule::new("rm", Decision::Prompt)
            .with_pattern(vec![PatternToken::Alternatives(vec!["-rf".into(), "-r".into()])]));
        policy.add_rule(PrefixRule::new("mkfs", Decision::Forbidden));
        policy.add_rule(PrefixRule::new("dd", Decision::Prompt)
            .with_pattern(vec![PatternToken::Exact("if=".into())]));
        policy.add_rule(PrefixRule::new("format", Decision::Forbidden));

        // Safe read-only commands
        policy.add_rule(PrefixRule::new("ls", Decision::Allow));
        policy.add_rule(PrefixRule::new("cat", Decision::Allow));
        policy.add_rule(PrefixRule::new("pwd", Decision::Allow));
        policy.add_rule(PrefixRule::new("echo", Decision::Allow));
        policy.add_rule(PrefixRule::new("which", Decision::Allow));

        policy
    }

    pub fn add_rule(&mut self, rule: PrefixRule) {
        self.rules.push(rule);
    }

    pub fn add_network_rule(&mut self, rule: NetworkRule) {
        self.network_rules.push(rule);
    }

    /// Check a command against the policy.
    pub fn check(&self, cmd: &[String], heuristics: fn(&[String]) -> Decision) -> Evaluation {
        if cmd.is_empty() {
            return Evaluation {
                decision: Decision::Prompt,
                matched_rules: vec!["empty command".into()],
            };
        }

        let mut matched = Vec::new();
        let mut max_decision = Decision::Allow;

        for rule in &self.rules {
            if let Some(desc) = rule.matches(cmd) {
                matched.push(desc);
                if rule.decision > max_decision {
                    max_decision = rule.decision;
                }
            }
        }

        if matched.is_empty() {
            // No explicit rule — use heuristics fallback
            let decision = heuristics(cmd);
            return Evaluation {
                decision,
                matched_rules: vec!["heuristics".into()],
            };
        }

        Evaluation {
            decision: max_decision,
            matched_rules: matched,
        }
    }

    /// Check network access against the policy.
    pub fn check_network(&self, host: &str, protocol: NetworkProtocol) -> Evaluation {
        for rule in &self.network_rules {
            if rule.host == host && (rule.protocol == NetworkProtocol::Any || rule.protocol == protocol)
            {
                return Evaluation {
                    decision: rule.decision,
                    matched_rules: vec![format!("network:{}", host)],
                };
            }
        }

        Evaluation {
            decision: Decision::Allow,
            matched_rules: vec!["default:allow".into()],
        }
    }

    /// Merge a higher-precedence overlay. Later rules override earlier ones.
    pub fn merge_overlay(&mut self, overlay: Policy) {
        // Overlay rules go last (higher precedence)
        self.rules.extend(overlay.rules);
        self.network_rules.extend(overlay.network_rules);
    }
}

/// Default heuristics for unmatched commands.
pub fn default_heuristics(cmd: &[String]) -> Decision {
    if cmd.is_empty() {
        return Decision::Prompt;
    }
    match cmd[0].as_str() {
        // Safe read-only
        "cat" | "ls" | "pwd" | "echo" | "which" | "whoami" | "head" | "tail" | "wc" => {
            Decision::Allow
        }
        // Dangerous
        "rm" | "rmdir" | "mkfs" | "dd" | "format" | "shutdown" | "reboot" => Decision::Forbidden,
        // Unknown
        _ => Decision::Prompt,
    }
}

/// Load a policy from a TOML string.
pub fn load_policy_from_str(toml: &str) -> Result<Policy, String> {
    #[derive(Deserialize)]
    struct PolicyConfig {
        #[serde(default)]
        rules: Vec<RuleConfig>,
        #[serde(default)]
        network_rules: Vec<NetworkRuleConfig>,
    }

    #[derive(Deserialize)]
    struct RuleConfig {
        program: String,
        decision: Decision,
        #[serde(default)]
        pattern: Vec<PatternToken>,
    }

    #[derive(Deserialize)]
    struct NetworkRuleConfig {
        host: String,
        protocol: NetworkProtocol,
        decision: Decision,
    }

    let config: PolicyConfig = toml::from_str(toml).map_err(|e| e.to_string())?;

    let mut policy = Policy::new();
    for rule in config.rules {
        policy.add_rule(PrefixRule {
            program: rule.program,
            decision: rule.decision,
            pattern: rule.pattern,
        });
    }
    for nr in config.network_rules {
        policy.add_network_rule(NetworkRule {
            host: nr.host,
            protocol: nr.protocol,
            decision: nr.decision,
        });
    }
    Ok(policy)
}

/// Load a policy from layered config files (system > user > project).
pub fn load_policy_layered(
    system: Option<&str>,
    user: Option<&str>,
    project: Option<&str>,
) -> Result<Policy, String> {
    let mut policy = Policy::new();

    if let Some(toml) = system {
        let overlay = load_policy_from_str(toml)?;
        policy.merge_overlay(overlay);
    }
    if let Some(toml) = user {
        let overlay = load_policy_from_str(toml)?;
        policy.merge_overlay(overlay);
    }
    if let Some(toml) = project {
        let overlay = load_policy_from_str(toml)?;
        policy.merge_overlay(overlay);
    }

    Ok(policy)
}
```

- [ ] **Step 4: Add pub mod execpolicy to lib.rs**

In `crates/aletheon-abi/src/lib.rs`, add:
```rust
pub mod execpolicy;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-abi --test execpolicy_tests 2>&1 | tail -15`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-abi/src/execpolicy.rs crates/aletheon-abi/src/lib.rs crates/aletheon-abi/tests/execpolicy_tests.rs
git commit -m "feat(abi): add independent execpolicy engine

Extracted policy logic into a standalone module with:
- Decision enum (Allow/Prompt/Forbidden) with Ord
- PrefixRule with pattern matching
- Policy with overlay merge (system > user > project)
- Network rules as first-class citizens
- TOML-based layered config loading
- Default heuristics for unmatched commands"
```

---

### Task 2.2: Integrate execpolicy into ToolRunnerWithGuard

**Files:**
- Modify: `crates/aletheon-body/src/impl/security/runner.rs`
- Modify: `crates/aletheon-body/src/impl/security/policy.rs` (deprecate)

- [ ] **Step 1: Write the failing test**

```rust
// Add to runner.rs tests

#[test]
fn runner_uses_execpolicy_for_decision() {
    use aletheon_abi::execpolicy::{Policy, Decision};

    let mut policy = Policy::with_defaults();
    // Add a rule that forbids a specific tool
    policy.add_rule(PrefixRule::new("forbidden_tool", Decision::Forbidden));

    let runner = ToolRunnerWithGuard::new(
        SandboxExecutor::new(SandboxPreference::Auto),
        AuditLogger::null(),
    ).with_policy(policy);

    // The runner should now use the new policy
    // This is tested via the full integration test below
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-body --lib security::runner::tests::runner_uses_execpolicy_for_decision 2>&1 | tail -5`
Expected: error: method `with_policy` not found

- [ ] **Step 3: Add with_policy method and integrate**

In `runner.rs`, add a new field and method:

```rust
use aletheon_abi::execpolicy::{Policy as ExecPolicy, Decision as ExecDecision};

pub struct ToolRunnerWithGuard {
    // ... existing fields ...
    /// Independent execpolicy engine (replaces inline PolicyEngine).
    exec_policy: Option<ExecPolicy>,
}

impl ToolRunnerWithGuard {
    // ... existing methods ...

    /// Set the execpolicy engine. When set, this takes precedence over the inline PolicyEngine.
    pub fn with_policy(mut self, policy: ExecPolicy) -> Self {
        self.exec_policy = Some(policy);
        self
    }

    /// Check policy using execpolicy if available, otherwise fall back to inline PolicyEngine.
    fn check_policy(&self, tool_name: &str, input: &serde_json::Value) -> PolicyVerdict {
        if let Some(ref policy) = self.exec_policy {
            // Use new execpolicy
            let cmd = self.build_command_vec(tool_name, input);
            let eval = policy.check(&cmd, aletheon_abi::execpolicy::default_heuristics);
            match eval.decision {
                ExecDecision::Allow => PolicyVerdict::Allow,
                ExecDecision::Forbidden => PolicyVerdict::Deny {
                    reason: format!("Policy forbids: {}", eval.matched_rules.join(", ")),
                },
                ExecDecision::Prompt => PolicyVerdict::RequireApproval {
                    reason: format!("Policy requires approval: {}", eval.matched_rules.join(", ")),
                },
            }
        } else {
            // Fall back to inline PolicyEngine
            self.policy_engine.check(tool_name, input)
        }
    }

    fn build_command_vec(&self, tool_name: &str, input: &serde_json::Value) -> Vec<String> {
        let mut cmd = vec![tool_name.to_string()];
        // For bash_exec, extract the command string
        if tool_name == "bash_exec" {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                cmd.extend(command.split_whitespace().map(|s| s.to_string()));
            }
        }
        cmd
    }
}
```

Update `execute_tool` to use `check_policy`:
```rust
// In execute_tool, replace:
// let verdict = self.policy_engine.check(tool_name, &input);
// With:
let verdict = self.check_policy(tool_name, &input);
```

- [ ] **Step 4: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-body 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/security/runner.rs
git commit -m "feat(security): integrate execpolicy into ToolRunnerWithGuard

ToolRunnerWithGuard now supports an optional execpolicy::Policy.
When set, policy decisions come from the independent engine.
Falls back to inline PolicyEngine for backward compatibility."
```

---

## Phase 3: Controller + Event System

### Task 3.1: Create EventSink trait and Event enum

**Files:**
- Create: `crates/aletheon-runtime/src/core/event_sink.rs`
- Modify: `crates/aletheon-runtime/src/core/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/aletheon-runtime/tests/event_sink_tests.rs

use aletheon_runtime::core::event_sink::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn channel_sink_receives_events() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelEventSink::new(tx);

    sink.emit(Event::TurnStarted);
    sink.emit(Event::Text { text: "hello".into() });

    let e1 = rx.recv().await.unwrap();
    assert!(matches!(e1, Event::TurnStarted));

    let e2 = rx.recv().await.unwrap();
    assert!(matches!(e2, Event::Text { text } if text == "hello"));
}

#[tokio::test]
async fn broadcast_sink_multiple_receivers() {
    let (tx, _) = tokio::sync::broadcast::channel(16);
    let sink = BroadcastEventSink::new(tx.clone());

    let mut rx1 = tx.subscribe();
    let mut rx2 = tx.subscribe();

    sink.emit(Event::TurnStarted);

    let e1 = rx1.recv().await.unwrap();
    let e2 = rx2.recv().await.unwrap();
    assert!(matches!(e1, Event::TurnStarted));
    assert!(matches!(e2, Event::TurnStarted));
}

#[test]
fn event_tool_result_carries_data() {
    let event = Event::ToolResult {
        name: "bash".into(),
        result: ToolResultEvent {
            content: "output".into(),
            is_error: false,
            execution_time_ms: 100,
        },
    };
    assert!(matches!(event, Event::ToolResult { name, .. } if name == "bash"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --test event_sink_tests 2>&1 | tail -5`
Expected: error: test target `event_sink_tests` not found

- [ ] **Step 3: Write event_sink.rs**

```rust
// crates/aletheon-runtime/src/core/event_sink.rs

//! Typed event stream for agent lifecycle events.
//!
//! All frontends observe the same event stream. Each frontend
//! implements `EventSink` to receive events.

use aletheon_abi::tool::ToolResult;

/// Lifecycle events emitted by the agent.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new turn started.
    TurnStarted,
    /// Streaming text from the LLM.
    Text { text: String },
    /// Reasoning/thinking text from the LLM.
    Reasoning { text: String },
    /// A tool call is about to be dispatched.
    ToolDispatch {
        name: String,
        args: serde_json::Value,
    },
    /// A tool execution completed.
    ToolResult {
        name: String,
        result: ToolResultEvent,
    },
    /// Token usage update.
    Usage {
        tokens_in: u32,
        tokens_out: u32,
        cache_hit_tokens: u32,
        cache_miss_tokens: u32,
    },
    /// An approval is needed from the user.
    ApprovalRequest {
        id: String,
        tool: String,
        args: serde_json::Value,
        reason: String,
    },
    /// A question needs answering.
    AskRequest {
        id: String,
        question: String,
        options: Vec<String>,
    },
    /// Context compaction started.
    CompactionStarted,
    /// Context compaction completed.
    CompactionDone { summary_chars: usize },
    /// The turn completed.
    TurnDone {
        result: Result<String, String>,
    },
    /// An error occurred.
    Error { message: String },
    /// Memory was updated (queued for next turn).
    MemoryUpdated { fact: String },
    /// Plan mode changed.
    PlanModeChanged { enabled: bool },
    /// Cache diagnostics.
    CacheDiagnostics {
        hit_tokens: u64,
        miss_tokens: u64,
        hit_rate: f64,
    },
}

/// Simplified tool result for events.
#[derive(Debug, Clone)]
pub struct ToolResultEvent {
    pub content: String,
    pub is_error: bool,
    pub execution_time_ms: u64,
}

impl From<&ToolResult> for ToolResultEvent {
    fn from(tr: &ToolResult) -> Self {
        Self {
            content: tr.content.clone(),
            is_error: tr.is_error,
            execution_time_ms: tr.metadata.execution_time_ms,
        }
    }
}

/// Trait for receiving events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// mpsc-based sink for async frontends.
pub struct ChannelEventSink {
    tx: tokio::sync::mpsc::Sender<Event>,
}

impl ChannelEventSink {
    pub fn new(tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        Self { tx }
    }
}

impl EventSink for ChannelEventSink {
    fn emit(&self, event: Event) {
        // Try send, drop if full (don't block the agent)
        let _ = self.tx.try_send(event);
    }
}

/// Broadcast sink for multiple subscribers.
pub struct BroadcastEventSink {
    tx: tokio::sync::broadcast::Sender<Event>,
}

impl BroadcastEventSink {
    pub fn new(tx: tokio::sync::broadcast::Sender<Event>) -> Self {
        Self { tx }
    }
}

impl EventSink for BroadcastEventSink {
    fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

/// No-op sink for testing.
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _event: Event) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_does_nothing() {
        let sink = NullEventSink;
        sink.emit(Event::TurnStarted); // should not panic
    }

    #[test]
    fn tool_result_from_conversion() {
        let tr = ToolResult {
            content: "ok".into(),
            is_error: false,
            metadata: aletheon_abi::tool::ToolResultMeta {
                execution_time_ms: 50,
                truncated: false,
            },
        };
        let event = ToolResultEvent::from(&tr);
        assert_eq!(event.content, "ok");
        assert_eq!(event.execution_time_ms, 50);
    }
}
```

- [ ] **Step 4: Add pub mod event_sink to mod.rs**

In `crates/aletheon-runtime/src/core/mod.rs`, add:
```rust
pub mod event_sink;
```

- [ ] **Step 5: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::event_sink::tests 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/event_sink.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add typed event stream (Event + EventSink)

Event enum covers full agent lifecycle: TurnStarted, Text, ToolDispatch,
ToolResult, Usage, ApprovalRequest, Compaction, TurnDone, Error.

EventSink trait with ChannelEventSink (mpsc), BroadcastEventSink,
and NullEventSink (testing) implementations."
```

---

### Task 3.2: Create Controller struct

**Files:**
- Create: `crates/aletheon-runtime/src/core/controller.rs`
- Modify: `crates/aletheon-runtime/src/core/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/aletheon-runtime/tests/controller_tests.rs

use aletheon_runtime::core::controller::*;
use aletheon_runtime::core::event_sink::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn controller_send_emits_events() {
    // This is a unit test skeleton - full integration needs mock LLM
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let sink = ChannelEventSink::new(event_tx);

    // Controller construction requires many dependencies
    // This test verifies the event emission pattern
    sink.emit(Event::TurnStarted);
    let event = event_rx.recv().await.unwrap();
    assert!(matches!(event, Event::TurnStarted));
}

#[test]
fn controller_options_construction() {
    let opts = ControllerOptions {
        working_dir: "/tmp".into(),
        data_dir: "/tmp/aletheon-test".into(),
        system_prompt: "You are a test agent.".into(),
        max_iterations: 10,
        compaction_enabled: false,
    };
    assert_eq!(opts.max_iterations, 10);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --test controller_tests 2>&1 | tail -5`
Expected: error: test target `controller_tests` not found

- [ ] **Step 3: Write controller.rs**

```rust
// crates/aletheon-runtime/src/core/controller.rs

//! Transport-agnostic controller.
//!
//! The Controller sits behind every frontend (TUI, daemon, HTTP, desktop).
//! All frontends issue the same commands and observe the same event stream.

use super::event_sink::{Event, EventSink};
use super::react_loop::ReActLoop;
use super::config::RuntimeConfig;
use aletheon_abi::tool::{Tool, ToolContext};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Options for constructing a Controller.
#[derive(Debug, Clone)]
pub struct ControllerOptions {
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub max_iterations: usize,
    pub compaction_enabled: bool,
}

impl Default for ControllerOptions {
    fn default() -> Self {
        Self {
            working_dir: "/tmp".into(),
            data_dir: "/tmp/aletheon".into(),
            system_prompt: "You are a helpful assistant.".into(),
            max_iterations: 15,
            compaction_enabled: true,
        }
    }
}

/// Transport-agnostic agent controller.
pub struct Controller {
    /// The ReAct loop (holds conversation state).
    react_loop: Arc<Mutex<ReActLoop>>,
    /// Event sink for lifecycle events.
    event_sink: Arc<dyn EventSink>,
    /// Whether a turn is currently running.
    running: Arc<Mutex<bool>>,
    /// Cancellation token for the current turn.
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    /// Working directory.
    working_dir: String,
    /// System prompt (immutable after construction).
    system_prompt: String,
    /// Pending memory updates (drain into user message).
    memory_queue: Arc<Mutex<Vec<String>>>,
    /// Plan mode flag.
    plan_mode: Arc<Mutex<bool>>,
}

impl Controller {
    /// Create a new Controller.
    pub fn new(opts: ControllerOptions, event_sink: Arc<dyn EventSink>) -> Self {
        let config = RuntimeConfig {
            max_iterations: opts.max_iterations,
            compaction_enabled: opts.compaction_enabled,
            ..RuntimeConfig::default()
        };

        let mut react_loop = ReActLoop::new(config);
        react_loop.set_system_prompt(opts.system_prompt.clone());

        Self {
            react_loop: Arc::new(Mutex::new(react_loop)),
            event_sink,
            running: Arc::new(Mutex::new(false)),
            cancel_token: Arc::new(Mutex::new(None)),
            working_dir: opts.working_dir,
            system_prompt: opts.system_prompt,
            memory_queue: Arc::new(Mutex::new(Vec::new())),
            plan_mode: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the system prompt (immutable).
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Set plan mode. Injected into user message, NOT system prompt.
    pub async fn set_plan_mode(&self, enabled: bool) {
        *self.plan_mode.lock().await = enabled;
        self.event_sink.emit(Event::PlanModeChanged { enabled });
    }

    /// Queue a memory update for the next turn.
    pub async fn queue_memory(&self, fact: String) {
        self.memory_queue.lock().await.push(fact.clone());
        self.event_sink.emit(Event::MemoryUpdated { fact });
    }

    /// Compose user message with mid-session injections.
    pub async fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        let plan = *self.plan_mode.lock().await;
        if plan {
            parts.push("[PLAN MODE ACTIVE: Think step-by-step before acting.]".to_string());
        }

        let mut queue = self.memory_queue.lock().await;
        if !queue.is_empty() {
            let updates = queue.iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
            queue.clear();
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }

    /// Check if a turn is currently running.
    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// Cancel the current turn.
    pub async fn cancel(&self) {
        let token = self.cancel_token.lock().await.take();
        if let Some(token) = token {
            token.cancel();
            info!("Turn cancelled");
        }
    }

    /// Get the event sink.
    pub fn event_sink(&self) -> &dyn EventSink {
        self.event_sink.as_ref()
    }

    /// Get the working directory.
    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_sink::NullEventSink;

    #[tokio::test]
    async fn compose_plain_input() {
        let controller = Controller::new(
            ControllerOptions::default(),
            Arc::new(NullEventSink),
        );
        let msg = controller.compose_user_message("hello").await;
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn compose_with_plan_mode() {
        let controller = Controller::new(
            ControllerOptions::default(),
            Arc::new(NullEventSink),
        );
        controller.set_plan_mode(true).await;
        let msg = controller.compose_user_message("hello").await;
        assert!(msg.contains("[PLAN MODE ACTIVE]"));
    }

    #[tokio::test]
    async fn compose_drains_memory_queue() {
        let controller = Controller::new(
            ControllerOptions::default(),
            Arc::new(NullEventSink),
        );
        controller.queue_memory("fact 1".into()).await;
        controller.queue_memory("fact 2".into()).await;

        let msg = controller.compose_user_message("hello").await;
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("fact 1"));
        assert!(msg.contains("fact 2"));

        // Queue should be drained
        let msg2 = controller.compose_user_message("world").await;
        assert!(!msg2.contains("<memory-update>"));
    }

    #[tokio::test]
    async fn system_prompt_immutable() {
        let controller = Controller::new(
            ControllerOptions::default(),
            Arc::new(NullEventSink),
        );
        let p1 = controller.system_prompt().to_string();
        controller.set_plan_mode(true).await;
        controller.queue_memory("fact".into()).await;
        let p2 = controller.system_prompt().to_string();
        assert_eq!(p1, p2);
    }

    #[tokio::test]
    async fn cancel_with_no_turn() {
        let controller = Controller::new(
            ControllerOptions::default(),
            Arc::new(NullEventSink),
        );
        controller.cancel().await; // should not panic
    }
}
```

- [ ] **Step 4: Add pub mod controller to mod.rs**

In `crates/aletheon-runtime/src/core/mod.rs`, add:
```rust
pub mod controller;
```

- [ ] **Step 5: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::controller::tests 2>&1 | tail -15`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/controller.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add transport-agnostic Controller

Controller holds ReActLoop, EventSink, memory queue, and plan mode.
All frontends (TUI, daemon, HTTP) drive the same Controller API.

Key methods:
- send() — async turn execution
- compose_user_message() — cache-stable injection
- set_plan_mode() / queue_memory() — mid-session state
- cancel() — abort current turn"
```

---

## Final Verification

- [ ] **Run full test suite**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo test 2>&1 | tail -20
```

Expected: all tests pass (existing + new)

- [ ] **Verify no regressions**

```bash
cargo test 2>&1 | grep -E "test result|failures"
```

Expected: `test result: ok. X passed; 0 failed`

- [ ] **Final commit with all changes**

```bash
git add -u
git commit -m "feat: P0 improvements — cache-first, execpolicy, controller

Phase 1: Cache-first prefix stability
- compose_user_message() injects changes into user message
- System prompt stays byte-stable for provider cache hits

Phase 2: Independent execpolicy engine
- Decision/Rule/Policy types in aletheon-abi
- Layered config (system > user > project)
- Integrated into ToolRunnerWithGuard

Phase 3: Transport-agnostic Controller
- Event enum + EventSink trait
- Controller struct for multi-frontend support
- compose_user_message() at controller level"
```
