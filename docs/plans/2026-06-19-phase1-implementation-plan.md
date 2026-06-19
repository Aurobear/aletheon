# Phase 1 Implementation Plan: Runtime ReAct Loop Wiring + Approval Gate

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This plan is structured for **multiple developer agents in parallel** — see the Dependency Graph and Parallel Batches sections.

**Goal:** Make the agent able to *act, safely* — fill `ReActLoop` into a real interleaved LLM+tool loop, add a human `ApprovalGate`, wire it into both `aletheon-exec` and the daemon `chat` path, so `create hello.txt` actually creates the file and risky (L2+) actions prompt for y/n.

**Architecture:** Interleaved ReAct (LLM → tool_use → execute-with-guard+approval → feed result back → repeat), reusing the verified `ToolRunnerWithGuard` pipeline. New `ApprovalGate` trait decouples the approval UI (terminal now, socket/TUI later). Reuse brain's existing `LlmResponse`/`StreamChunk`/`ToolDefinition` types — no new abi types. CLI/TUI stay in `body` (no new crate).

**Tech Stack:** Rust, async-trait, tokio, serde_json. Workspace crates: `aletheon-body`, `aletheon-runtime`, `aletheon-brain`, `aletheon-abi`.

**Design spec:** [2026-06-19-runtime-react-loop-wiring-design.md](./2026-06-19-runtime-react-loop-wiring-design.md)

---

## File Structure & Owner Boundaries

Each agent owns a disjoint set of files (no two agents write the same file), enabling true parallelism.

| Agent | Owns (writes) | Responsibility |
|-------|---------------|----------------|
| **A — Approval** | `crates/aletheon-body/src/impl/security/approval.rs` (NEW), `crates/aletheon-body/src/impl/security/runner.rs`, `crates/aletheon-body/src/impl/security/mod.rs` | `ApprovalGate` trait, `ApprovalRequest`/`ApprovalDecision`, `TerminalApprovalGate`, `AutoApproveGate`/`AutoDenyGate`; wire the gate into `ToolRunnerWithGuard`. |
| **B — Loop** | `crates/aletheon-runtime/src/core/react_loop.rs`, `crates/aletheon-runtime/src/core/orchestrator.rs` | Upgrade `ReActLoop` from counter to interleaved LLM+tool loop; re-scope `AletheonRuntime::process()` to drive it. |
| **C — Exec** | `crates/binaries/aletheon-exec/src/main.rs` | Load `~/.aletheon/.env`; replace raw tool exec with `ToolRunnerWithGuard` + `TerminalApprovalGate`. |
| **D — Daemon** | `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Wire the `chat` handler to drive the interleaved loop with real tools + the (currently dropped) `tool_runner`, using a conservative gate (no UI regression). |

**Shared read-only (no agent modifies):** `aletheon-abi/src/tool.rs`, `aletheon-abi/src/message.rs`, `aletheon-brain/src/impl/llm/provider.rs`.

---

## Dependency Graph

```
Batch 1 (parallel, no deps):
  Agent A: Task A1 (ApprovalGate types) → A2 (gates) → A3 (runner wiring)
  Agent B: Task B1 (ReActLoop.run loop) → B2 (orchestrator.process re-scope)

Batch 2 (after A3 + B2 land on the branch):
  Agent C: Task C1 (.env) → C2 (guard+approval in exec) → C3 (exec e2e test)   [needs A3]
  Agent D: Task D1 (chat → interleaved loop)                                    [needs A3 + B2]

Batch 3 (after C + D):
  Task E1 (workspace build + full test) → E2 (defining acceptance test)
```

`A3` (runner wiring) is the critical shared dependency: both C and D consume the new
`ApprovalGate`. B is independent of A until D. So **A and B run fully in parallel in
Batch 1**; **C and D run in parallel in Batch 2**.

---

## Parallel Batches (for the coordinator)

- **Batch 1:** dispatch Agent A and Agent B concurrently. Each commits to the shared
  feature branch when its tasks pass. Barrier: both must land before Batch 2.
- **Batch 2:** dispatch Agent C and Agent D concurrently.
- **Batch 3:** single integration agent runs E1+E2.

All agents work on branch `auro/feat/20260619-runtime-react-loop-wiring`. Each task ends
with a commit so progress is visible and conflicts surface early.

---

## Agent A — Approval Gate

### Task A1: ApprovalGate types

**Files:**
- Create: `crates/aletheon-body/src/impl/security/approval.rs`
- Modify: `crates/aletheon-body/src/impl/security/mod.rs`

- [ ] **Step 1: Write the failing test** — append to `approval.rs` (created in Step 3, test lives at bottom of the new file):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auto_deny_gate_denies() {
        let gate = AutoDenyGate;
        let req = ApprovalRequest {
            tool: "bash_exec".into(),
            action_summary: "rm -rf /tmp/x".into(),
            risk_level: "high".into(),
            detail: None,
        };
        assert_eq!(gate.request(&req).await, ApprovalDecision::Deny);
    }

    #[tokio::test]
    async fn auto_approve_gate_approves() {
        let gate = AutoApproveGate;
        let req = ApprovalRequest {
            tool: "file_write".into(),
            action_summary: "write hello.txt".into(),
            risk_level: "low".into(),
            detail: None,
        };
        assert_eq!(gate.request(&req).await, ApprovalDecision::Approve);
    }
}
```

- [ ] **Step 2: Write the implementation** — create `crates/aletheon-body/src/impl/security/approval.rs`:

```rust
//! Human-in-the-loop approval gate for risky tool execution.
//!
//! Decouples the approval *decision channel* (terminal, socket, auto) from the
//! security runner. The runner asks the gate before executing L2+ tools; the
//! gate returns the user's decision. Fail-safe: any error/timeout upstream
//! should map to `Deny`.

use async_trait::async_trait;

/// A request for the user to approve a single tool action.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Tool name, e.g. "bash_exec".
    pub tool: String,
    /// One-line human-readable summary, e.g. "bash: rm -rf /tmp/x".
    pub action_summary: String,
    /// Risk descriptor, e.g. "low" | "medium" | "high".
    pub risk_level: String,
    /// Optional full command / diff for the user to inspect.
    pub detail: Option<String>,
}

/// The user's decision on an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Execute this action once.
    Approve,
    /// Reject this action.
    Deny,
    /// Approve this action and auto-approve the same tool for the rest of the session.
    ApproveForSession,
}

/// Abstraction over how approval is requested from the user.
///
/// Implementations: `TerminalApprovalGate` (stdin/stdout y/n), `AutoApproveGate` /
/// `AutoDenyGate` (tests and conservative defaults). A socket/TUI gate arrives in
/// Phase 2.
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    /// Request a decision for the given action. Implementations must never panic;
    /// on any internal failure they should return `ApprovalDecision::Deny`.
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision;
}

/// Always approves. For tests and trusted/automated contexts.
pub struct AutoApproveGate;

#[async_trait]
impl ApprovalGate for AutoApproveGate {
    async fn request(&self, _req: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approve
    }
}

/// Always denies. The conservative default (preserves current "deny L2+ in automated
/// mode" behavior) and a test double.
pub struct AutoDenyGate;

#[async_trait]
impl ApprovalGate for AutoDenyGate {
    async fn request(&self, _req: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}
```

- [ ] **Step 3: Register the module** — in `crates/aletheon-body/src/impl/security/mod.rs`, add the module declaration and re-exports. First inspect the file to place these next to the existing `pub mod ...;` lines:

```rust
pub mod approval;
pub use approval::{ApprovalGate, ApprovalRequest, ApprovalDecision, AutoApproveGate, AutoDenyGate, TerminalApprovalGate};
```

(`TerminalApprovalGate` is added in Task A2; including it here now is fine because A2
lands before this agent's work is considered complete. If the compiler complains
between A1 and A2, temporarily omit `TerminalApprovalGate` from the re-export and add it
in A2.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p aletheon-body security::approval -- --nocapture`
Expected: `auto_deny_gate_denies` and `auto_approve_gate_approves` PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/security/approval.rs crates/aletheon-body/src/impl/security/mod.rs
git commit -m "feat(security): add ApprovalGate trait + auto gates"
```

---

### Task A2: TerminalApprovalGate

**Files:**
- Modify: `crates/aletheon-body/src/impl/security/approval.rs`

- [ ] **Step 1: Add the implementation** — append to `approval.rs` (above the `#[cfg(test)]` module):

```rust
/// Approval gate that prompts on the controlling terminal (stdin/stdout).
///
/// Used by the single-process `aletheon-exec` path. Reads one line:
/// `y` = Approve, `a` = ApproveForSession, anything else (incl. EOF) = Deny (fail-safe).
pub struct TerminalApprovalGate;

#[async_trait]
impl ApprovalGate for TerminalApprovalGate {
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let mut stdout = tokio::io::stdout();
        let prompt = format!(
            "\n\u{26a0}  Approval required [{}] {}\n   {}\n   Approve? [y]es / [a]lways / [N]o: ",
            req.risk_level, req.tool, req.action_summary,
        );
        if stdout.write_all(prompt.as_bytes()).await.is_err() {
            return ApprovalDecision::Deny;
        }
        let _ = stdout.flush().await;

        let mut line = String::new();
        let mut reader = BufReader::new(tokio::io::stdin());
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => ApprovalDecision::Deny, // EOF or error → fail-safe deny
            Ok(_) => match line.trim().to_lowercase().as_str() {
                "y" | "yes" => ApprovalDecision::Approve,
                "a" | "always" => ApprovalDecision::ApproveForSession,
                _ => ApprovalDecision::Deny,
            },
        }
    }
}
```

- [ ] **Step 2: Verify it compiles** (no automated test — it reads a real TTY; covered by the manual acceptance test in E2)

Run: `cargo build -p aletheon-body`
Expected: builds clean, no new warnings about `TerminalApprovalGate`.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-body/src/impl/security/approval.rs
git commit -m "feat(security): add TerminalApprovalGate (stdin y/a/N)"
```

---

### Task A3: Wire ApprovalGate into ToolRunnerWithGuard

**Files:**
- Modify: `crates/aletheon-body/src/impl/security/runner.rs`

- [ ] **Step 1: Write the failing test** — append to `runner.rs` inside a `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::r#impl::security::approval::{AutoApproveGate, AutoDenyGate};
    use crate::r#impl::security::audit::AuditLogger;
    use aletheon_abi::tool::{Tool, ToolContext, ToolResult, ToolResultMeta, PermissionLevel};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct DummyL2Tool;
    #[async_trait]
    impl Tool for DummyL2Tool {
        fn name(&self) -> &str { "dummy_l2" }
        fn description(&self) -> &str { "an L2 tool for testing approval" }
        fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
        fn permission_level(&self) -> PermissionLevel { PermissionLevel::L2 }
        async fn execute(&self, _p: serde_json::Value, _c: &ToolContext) -> ToolResult {
            ToolResult { content: "executed".into(), is_error: false, metadata: ToolResultMeta::default() }
        }
        fn boxed_clone(&self) -> Box<dyn Tool> { Box::new(DummyL2Tool) }
    }

    fn ctx() -> ToolContext {
        ToolContext { working_dir: std::env::temp_dir(), session_id: "t".into() }
    }

    #[tokio::test]
    async fn l2_denied_by_gate_is_blocked() {
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(AuditLogger::noop())
            .with_approval_gate(Arc::new(AutoDenyGate));
        let res = runner.execute_tool(&DummyL2Tool, serde_json::json!({}), &ctx(), "turn1").await;
        assert!(matches!(res, Err(ToolError::PolicyDenied { .. })));
    }

    #[tokio::test]
    async fn l2_approved_by_gate_runs() {
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(AuditLogger::noop())
            .with_approval_gate(Arc::new(AutoApproveGate));
        let res = runner.execute_tool(&DummyL2Tool, serde_json::json!({}), &ctx(), "turn1").await;
        // L2 with no "command" arg falls to direct execute → "executed"
        assert!(res.is_ok());
    }
}
```

> If `AuditLogger::noop()` does not exist, the test should construct an `AuditLogger`
> the same way `with_default_sandbox`'s existing call sites do — check
> `crates/aletheon-body/src/impl/security/audit.rs` for the available constructor and
> use it. Do not invent an API.

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p aletheon-body approval_tests 2>&1 | head`
Expected: FAIL — `with_approval_gate` does not exist yet.

- [ ] **Step 3: Add the gate field + builder** — modify `runner.rs`:

  3a. Add import near the top (after line 10):
  ```rust
  use std::sync::Arc;
  use super::approval::{ApprovalGate, ApprovalRequest, ApprovalDecision, AutoDenyGate};
  ```

  3b. Add a field to `struct ToolRunnerWithGuard` (after `risk_classifier`):
  ```rust
      /// Approval gate consulted before executing tools that require approval.
      /// Defaults to AutoDenyGate (conservative: preserves prior "deny L2+" behavior).
      approval_gate: Arc<dyn ApprovalGate>,
      /// Tool names approved for the rest of the session (via ApproveForSession).
      session_approvals: std::collections::HashSet<String>,
  ```

  3c. In `fn new(...)`, initialize the two new fields (inside the returned `Self { ... }`):
  ```rust
          approval_gate: Arc::new(AutoDenyGate),
          session_approvals: std::collections::HashSet::new(),
  ```

  3d. Add a builder method in `impl ToolRunnerWithGuard` (after `with_default_sandbox`):
  ```rust
      /// Set the approval gate used for actions that require approval.
      pub fn with_approval_gate(mut self, gate: Arc<dyn ApprovalGate>) -> Self {
          self.approval_gate = gate;
          self
      }
  ```

- [ ] **Step 4: Replace the auto-deny logic with a gate consultation** — in `execute_tool`, replace the entire `PolicyVerdict::RequireApproval { reason }` arm (currently lines ~94-102) with:

```rust
            PolicyVerdict::RequireApproval { reason } => {
                // Consult the approval gate for L2+ actions instead of auto-denying.
                if tool.permission_level() >= PermissionLevel::L2 {
                    if self.session_approvals.contains(tool_name) {
                        // Previously approved-for-session; allow.
                    } else {
                        let summary = input
                            .get("command")
                            .and_then(|v| v.as_str())
                            .map(|c| format!("{}: {}", tool_name, c))
                            .unwrap_or_else(|| format!("{}: {}", tool_name, input));
                        let req = ApprovalRequest {
                            tool: tool_name.to_string(),
                            action_summary: summary,
                            risk_level: format!("{:?}", tool.permission_level()),
                            detail: Some(input.to_string()),
                        };
                        match self.approval_gate.request(&req).await {
                            ApprovalDecision::Approve => {}
                            ApprovalDecision::ApproveForSession => {
                                self.session_approvals.insert(tool_name.to_string());
                            }
                            ApprovalDecision::Deny => {
                                self.log_audit(tool_name, &input, tool.permission_level(), turn_id, None, &start, "approval_denied").await;
                                return Err(ToolError::PolicyDenied {
                                    reason: format!("{}: denied by approval gate", reason),
                                });
                            }
                        }
                    }
                }
            }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p aletheon-body approval_tests -- --nocapture`
Expected: `l2_denied_by_gate_is_blocked` PASS, `l2_approved_by_gate_runs` PASS.

- [ ] **Step 6: Run the full body suite (no regression)**

Run: `cargo test -p aletheon-body`
Expected: all pass (the prior "deny L2+ in automated mode" behavior is preserved because
the default gate is `AutoDenyGate`).

- [ ] **Step 7: Commit**

```bash
git add crates/aletheon-body/src/impl/security/runner.rs
git commit -m "feat(security): consult ApprovalGate for L2+ instead of auto-deny"
```

---

## Agent B — Interleaved ReAct Loop

### Task B1: ReActLoop.run — real interleaved loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Write the failing test** — append to `react_loop.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::message::{ContentBlock, Message, Role};
    use aletheon_brain::r#impl::llm::provider::{LlmProvider, LlmResponse, StopReason, Usage, LlmStream};
    use aletheon_abi::ToolDefinition;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // Mock LLM: first call emits a tool_use, second call ends the turn.
    struct ScriptedLlm { calls: Mutex<usize> }
    #[async_trait]
    impl LlmProvider for ScriptedLlm {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Ok(LlmResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".into(), name: "echo_tool".into(),
                        input: serde_json::json!({"text": "hi"}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(), cache_hit_tokens: 0, cache_miss_tokens: 0,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text { text: "done: hi".into() }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(), cache_hit_tokens: 0, cache_miss_tokens: 0,
                })
            }
        }
        async fn complete_stream(&self, m: &[Message], t: &[ToolDefinition]) -> anyhow::Result<LlmStream> {
            let _ = self.complete(m, t).await?; unreachable!()
        }
        fn name(&self) -> &str { "scripted" }
        fn max_context_length(&self) -> usize { 100_000 }
    }

    #[tokio::test]
    async fn interleaved_loop_executes_tool_then_finishes() {
        let cfg = RuntimeConfig { max_iterations: 5, session_id: "t".into(), learning_enabled: false, compaction_enabled: false };
        let mut lp = ReActLoop::new(cfg);
        let llm = ScriptedLlm { calls: Mutex::new(0) };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let executed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executed2 = executed.clone();

        let out = lp.run(
            "make hi",
            &llm,
            &tool_defs,
            |_id: &str, name: &str, _input: &serde_json::Value| {
                let executed = executed2.clone();
                let name = name.to_string();
                async move {
                    executed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (format!("ran {}", name), false)
                }
            },
        ).await.unwrap();

        assert_eq!(executed.load(std::sync::atomic::Ordering::SeqCst), 1, "tool ran exactly once");
        assert!(out.contains("done"), "final text returned: {out}");
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p aletheon-runtime react_loop 2>&1 | head`
Expected: FAIL — `ReActLoop::run` does not exist.

- [ ] **Step 3: Add `messages` field and the `run` method** — modify `react_loop.rs`:

  3a. Update imports at top:
  ```rust
  use aletheon_abi::body::Action;
  use aletheon_abi::message::{ContentBlock, Message, Role};
  use aletheon_abi::self_field::{Intent, IntentSource};
  use aletheon_abi::ToolDefinition;
  use aletheon_brain::r#impl::llm::provider::{LlmProvider, StopReason};
  use crate::core::config::RuntimeConfig;
  use std::future::Future;
  use tracing::{debug, warn};
  ```

  3b. Add `messages` to the struct:
  ```rust
  pub struct ReActLoop {
      config: RuntimeConfig,
      iteration: usize,
      messages: Vec<Message>,
  }
  ```
  and initialize `messages: Vec::new()` in `new`.

  3c. Update `reset` to also clear messages:
  ```rust
      pub fn reset(&mut self) {
          self.iteration = 0;
          self.messages.clear();
      }
  ```

  3d. Add the interleaved loop method (in `impl ReActLoop`):
  ```rust
      /// Run the interleaved ReAct loop: call the LLM with tools, execute any
      /// requested tools via `execute_tool`, feed results back, and repeat until
      /// the LLM stops requesting tools or `max_iterations` is reached.
      ///
      /// `execute_tool(id, name, input) -> (content, is_error)` performs one tool
      /// call. The caller supplies it so the loop stays free of body/security deps;
      /// the daemon and exec inject a closure backed by `ToolRunnerWithGuard`.
      pub async fn run<L, F, Fut>(
          &mut self,
          user_input: &str,
          llm: &L,
          tool_defs: &[ToolDefinition],
          execute_tool: F,
      ) -> anyhow::Result<String>
      where
          L: LlmProvider + ?Sized,
          F: Fn(&str, &str, &serde_json::Value) -> Fut,
          Fut: Future<Output = (String, bool)>,
      {
          self.messages.push(Message::user(user_input));

          while self.should_continue() {
              self.advance();
              let response = llm.complete(&self.messages, tool_defs).await?;

              let mut text_parts = Vec::new();
              let mut tool_calls = Vec::new();
              for block in &response.content {
                  match block {
                      ContentBlock::Text { text } => text_parts.push(text.clone()),
                      ContentBlock::ToolUse { id, name, input } => {
                          tool_calls.push((id.clone(), name.clone(), input.clone()));
                      }
                      _ => {}
                  }
              }

              if tool_calls.is_empty() || matches!(response.stop_reason, StopReason::EndTurn) {
                  let final_text = text_parts.join("\n");
                  self.messages.push(Message::assistant(&final_text));
                  return Ok(final_text);
              }

              // Record the assistant turn (text + tool_use blocks) verbatim.
              self.messages.push(Message { role: Role::Assistant, content: response.content.clone() });

              // Execute each requested tool and feed results back.
              for (id, name, input) in &tool_calls {
                  debug!(tool = name.as_str(), "ReActLoop executing tool");
                  let (content, is_error) = execute_tool(id, name, input).await;
                  if is_error {
                      warn!(tool = name.as_str(), "tool returned error");
                  }
                  self.messages.push(Message::tool_result(id, &content, is_error));
              }
          }

          warn!(max = self.config.max_iterations, "ReActLoop hit max_iterations");
          Ok(self.messages.iter().rev().find_map(|m| {
              m.content.iter().find_map(|b| match b {
                  ContentBlock::Text { text } => Some(text.clone()),
                  _ => None,
              })
          }).unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations)))
      }
  ```

- [ ] **Step 4: Add the brain dependency** — `aletheon-runtime/Cargo.toml` already depends on
  `aletheon-brain` (the daemon uses it). Verify:

  Run: `grep aletheon-brain crates/aletheon-runtime/Cargo.toml`
  Expected: a dependency line is present. If absent, add `aletheon-brain = { path = "../aletheon-brain" }` under `[dependencies]`.

- [ ] **Step 5: Run the test**

Run: `cargo test -p aletheon-runtime react_loop -- --nocapture`
Expected: `interleaved_loop_executes_tool_then_finishes` PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs crates/aletheon-runtime/Cargo.toml
git commit -m "feat(runtime): ReActLoop.run interleaved LLM+tool loop"
```

---

### Task B2: AletheonRuntime::process drives the loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/orchestrator.rs`

- [ ] **Step 1: Add a loop-driving method** — add a new method to `impl AletheonRuntime`
  that drives `ReActLoop::run` for the common Cognitive/Volitional path. This is additive;
  it does not remove the existing `process()` (kept for the plan-mode/closure callers).

```rust
    /// Process input via the interleaved ReAct loop.
    ///
    /// This is the agentic entry point: it runs SelfField review once (via
    /// `review_fn`), then drives `ReActLoop::run` with the given LLM, tool
    /// definitions, and per-tool executor. Replaces the daemon's old single
    /// `llm.complete(&[])`.
    pub async fn process_react<L, R, F, Fut>(
        &mut self,
        input: &str,
        ctx: &Context,
        review_fn: R,
        llm: &L,
        tool_defs: &[aletheon_abi::ToolDefinition],
        execute_tool: F,
    ) -> Result<String>
    where
        L: aletheon_brain::r#impl::llm::provider::LlmProvider + ?Sized,
        R: Fn(&Intent, &Context) -> Result<Verdict>,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = (String, bool)>,
    {
        self.react_loop.reset();
        let intent = self.react_loop.build_intent(input);
        let verdict = review_fn(&intent, ctx)?;
        debug!("SelfField verdict: {:?}", verdict);
        if let Verdict::Deny { reason } = verdict {
            return Ok(format!("Denied by SelfField: {}", reason));
        }
        self.react_loop.run(input, llm, tool_defs, execute_tool).await
    }
```

  Ensure the imports at the top of `orchestrator.rs` include `debug` (already present) and
  that `aletheon_abi::ToolDefinition` resolves (it re-exports from `message`/lib — verify
  with `grep -n "pub use.*ToolDefinition" crates/aletheon-abi/src/lib.rs`; if it's only at
  `aletheon_abi::message::ToolDefinition`, use that path).

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p aletheon-runtime`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/core/orchestrator.rs
git commit -m "feat(runtime): AletheonRuntime::process_react drives interleaved loop"
```

---

## Agent C — aletheon-exec (needs A3)

### Task C1: Load ~/.aletheon/.env

**Files:**
- Modify: `crates/binaries/aletheon-exec/src/main.rs`

- [ ] **Step 1: Add a local dotenv loader + call it first in `run`** — add this function near
  the top of `main.rs` (after the imports):

```rust
/// Minimal KEY=VALUE .env loader (no shell expansion). Mirrors the daemon's loader so
/// exec resolves provider API keys the same way the daemon does. Does not override
/// already-set process env vars.
fn load_dotenv(path: &std::path::Path) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if std::env::var(k).is_err() {
                std::env::set_var(k, v);
            }
        }
    }
}
```

  Then, as the **first** statement inside `async fn run(args: Args)` (before loading config):

```rust
    // Load ~/.aletheon/.env so provider API keys resolve (the daemon does this too).
    if let Some(home) = std::env::var_os("HOME") {
        load_dotenv(&std::path::Path::new(&home).join(".aletheon").join(".env"));
    }
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p aletheon-exec`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add crates/binaries/aletheon-exec/src/main.rs
git commit -m "fix(exec): load ~/.aletheon/.env for provider auth"
```

---

### Task C2: Route exec tools through ToolRunnerWithGuard + TerminalApprovalGate

**Files:**
- Modify: `crates/binaries/aletheon-exec/src/main.rs`

- [ ] **Step 1: Add imports** at the top of `main.rs`:

```rust
use std::sync::Arc;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::security::approval::{TerminalApprovalGate, ApprovalGate};
use aletheon_body::r#impl::security::audit::AuditLogger;
```

> Verify the `AuditLogger` constructor used here matches one that exists (check
> `audit.rs`); if `ToolRunnerWithGuard::with_default_sandbox` requires a specific
> `AuditLogger`, build it the same way the daemon's `handler.rs` does.

- [ ] **Step 2: Construct a guarded runner once, before the agent loop** — after the
  `tool_registry` is created (around the current line 126):

```rust
    // Guarded runner with terminal approval for risky (L2+) tools.
    let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(AuditLogger::default())
        .with_approval_gate(approval);
    let turn_id = uuid::Uuid::new_v4().to_string();
    runner.on_new_turn(&turn_id);
```

> If `AuditLogger::default()` is not available, use the constructor present in
> `audit.rs` (do not invent one).

- [ ] **Step 3: Replace the raw tool execution** — in the `StopReason::ToolUse` arm,
  replace the body of the `if let Some(tool) = tool_registry.get(name)` block so it goes
  through the guarded runner instead of `tool.execute(...)` directly:

```rust
                        let tool_result = if let Some(tool) = tool_registry.get(name) {
                            let result = runner
                                .run(tool.as_ref(), input.clone(), &tool_ctx, &turn_id)
                                .await;
                            if result.is_error {
                                warn!(tool = %name, error = %result.content, "Tool failed/denied");
                            } else {
                                info!(tool = %name, "Tool succeeded");
                            }
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: result.content,
                                is_error: result.is_error,
                            }
                        } else {
                            warn!(tool = %name, "Unknown tool");
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: format!("Error: Unknown tool '{}'", name),
                                is_error: true,
                            }
                        };
```

  (`runner.run(...)` returns `ToolResult` directly and routes through policy → approval →
  loop detector → sandbox/exec → audit. Denied L2+ actions come back as `is_error: true`
  with a "denied by approval gate" message, which feeds back to the LLM.)

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p aletheon-exec`
Expected: builds clean, no new warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/binaries/aletheon-exec/src/main.rs
git commit -m "feat(exec): route tools through ToolRunnerWithGuard + terminal approval"
```

---

### Task C3: Exec end-to-end smoke (file actually created)

**Files:**
- Test only (manual + scripted), no source changes.

- [ ] **Step 1: Build**

Run: `cargo build -p aletheon-exec`
Expected: success.

- [ ] **Step 2: Run the defining scenario** (requires a working provider key in
  `~/.aletheon/.env`, e.g. `MIMO_API_KEY`):

```bash
cd /tmp && rm -f hello.txt
/home/aurobear/Bear-ws/work/aletheon2/target/debug/aletheon-exec \
  --prompt "Create a file hello.txt in the current directory containing 'hi'. Use the bash tool." \
  --max-turns 6 -d /tmp
ls -la /tmp/hello.txt && cat /tmp/hello.txt
```

Expected: command exits 0; `hello.txt` **exists** and contains `hi`. (Before this plan,
the file was never created — this is the regression the whole phase targets.)

- [ ] **Step 3: Record the result** in the PR description (paste the `ls`/`cat` output).
  No commit (no source change), but note completion in the task tracker.

---

## Agent D — Daemon chat wiring (needs A3 + B2)

### Task D1: Wire the chat handler to the interleaved loop

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Make the dropped runner real** — at `handler.rs:131`, change:
  ```rust
          let _tool_runner = ToolRunnerWithGuard::new(sandbox, audit_logger);
  ```
  to construct it with the conservative default gate and store it on the handler. Because
  the handler is `Clone` and shared, wrap it for interior mutability:

  1a. Add a field to `struct RequestHandler` (near `self_field`):
  ```rust
      /// Guarded tool runner (policy → approval → loop detector → sandbox → audit).
      /// Conservative default gate (AutoDeny for L2+) until the Phase 2 socket gate.
      tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
  ```

  1b. In `new(...)`, replace the dropped `_tool_runner` line with:
  ```rust
          let tool_runner = Arc::new(Mutex::new(
              ToolRunnerWithGuard::new(sandbox, audit_logger)
          ));
  ```
  and add `tool_runner: tool_runner.clone(),` (or move) to the `RequestHandler { ... }`
  constructor literal. The default approval gate is already `AutoDenyGate`, so L2+ stays
  blocked — no safety regression vs. today.

- [ ] **Step 2: Build the tool definitions + executor closure and replace the empty
  `complete(&[])`** — in the `chat` arm, replace the block currently at line ~500
  (`match self.llm.complete(&messages, &[]).await { ... }`) with a call that runs the
  interleaved loop. Concretely:

  2a. Before the call, build tool defs from the handler's `tools` registry (the
  `ToolRegistry` already constructed in `new` — store it on the handler the same way as
  `tool_runner` if not already accessible; if a `tools: Arc<ToolRegistry>` field is needed,
  add it in `new` alongside `tool_runner`).

  ```rust
                let tool_defs = self.tools.definitions();
                let runner = self.tool_runner.clone();
                let tools = self.tools.clone();
                let working_dir = std::env::current_dir().unwrap_or_default();
                let session_id = self.session_manager.lock().await.session_id.clone();
                let exec_ctx = aletheon_abi::tool::ToolContext { working_dir, session_id: session_id.clone() };

                let execute_tool = move |_id: &str, name: &str, input: &serde_json::Value| {
                    let runner = runner.clone();
                    let tools = tools.clone();
                    let exec_ctx = exec_ctx.clone();
                    let name = name.to_string();
                    let input = input.clone();
                    async move {
                        match tools.get(&name) {
                            Some(tool) => {
                                let mut r = runner.lock().await;
                                let res = r.run(tool.as_ref(), input, &exec_ctx, "chat-turn").await;
                                (res.content, res.is_error)
                            }
                            None => (format!("Unknown tool: {}", name), true),
                        }
                    }
                };
  ```

  2b. Drive the loop. The handler already holds `self.llm: Arc<dyn LlmProvider>`. Build a
  fresh `ReActLoop`/`AletheonRuntime` for the turn (or reuse `state.runtime`):

  ```rust
                let mut rt = crate::core::orchestrator::AletheonRuntime::new(
                    self.state.lock().await.runtime.config().clone()
                );
                let sf_ctx2 = sf_ctx.clone();
                let result = rt.process_react(
                    &effective_message,
                    &sf_ctx2,
                    |_intent: &Intent, _c: &aletheon_abi::context::Context| Ok::<_, anyhow::Error>(Verdict::Allow),
                    self.llm.as_ref(),
                    &tool_defs,
                    execute_tool,
                ).await;
  ```

  > Note: SelfField review already ran earlier in this `chat` arm (lines 388–413), so the
  > inner `review_fn` here returns `Allow` to avoid double-gating. The real per-tool
  > approval happens inside `runner.run` via the gate.

  2c. Map `result` to the existing response/narration/PostTurn-hook code that follows
  (it currently consumes `text`); set `let text = result.unwrap_or_else(|e| format!("error: {e}"));`
  and feed it into the existing `sf_narrate` / `push_assistant` / PostTurn path unchanged.

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p aletheon-runtime`
Expected: builds clean. Resolve any borrow/move issues by cloning `Arc`s as shown.

- [ ] **Step 4: Run the runtime suite (no regression)**

Run: `cargo test -p aletheon-runtime`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(daemon): wire chat handler to interleaved ReAct loop with tools"
```

---

## Batch 3 — Integration & Acceptance

### Task E1: Workspace build + full test suite

- [ ] **Step 1: Format + build**

Run: `cargo fmt --all && cargo build --workspace`
Expected: clean build, no errors.

- [ ] **Step 2: Full test suite (no regression below the current 1234)**

Run: `cargo test --workspace 2>&1 | grep -E "test result" | awk '{p+=$4; f+=$6} END {print "passed",p,"failed",f}'`
Expected: `failed 0`, `passed >= 1234 + new tests`.

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no errors. Fix any new warnings introduced by this phase.

- [ ] **Step 4: Commit any fmt/clippy fixups**

```bash
git add -A && git commit -m "chore: fmt + clippy fixups for phase 1"
```

### Task E2: Defining acceptance test (the two behaviors this whole phase exists for)

- [ ] **Step 1: "can act" — file is actually created (exec path)**

```bash
cd /tmp && rm -f hello.txt
/home/aurobear/Bear-ws/work/aletheon2/target/debug/aletheon-exec \
  --prompt "Create hello.txt containing 'hi' using the bash tool." --max-turns 6 -d /tmp
test -f /tmp/hello.txt && grep -q hi /tmp/hello.txt && echo "ACT-PASS" || echo "ACT-FAIL"
```
Expected: `ACT-PASS`.

- [ ] **Step 2: "safe" — an L2 action prompts y/n and aborts on `n`**

```bash
cd /tmp && rm -f deleteme.txt && echo x > deleteme.txt
printf 'n\n' | /home/aurobear/Bear-ws/work/aletheon2/target/debug/aletheon-exec \
  --prompt "Delete the file /tmp/deleteme.txt using rm." --max-turns 4 -d /tmp
test -f /tmp/deleteme.txt && echo "SAFE-PASS (file survived denial)" || echo "SAFE-FAIL (file deleted despite n)"
```
Expected: the approval prompt appears, and because we answered `n`, `deleteme.txt`
**still exists** → `SAFE-PASS`.

> If `rm` is classified below L2 by the policy engine, adjust the policy or use a tool
> the policy marks L2+ (check `security/policy.rs` defaults). The behavior under test is:
> a policy-`RequireApproval` action, answered `n`, does not execute.

- [ ] **Step 3: Record both outputs in the PR description.**

---

## Self-Review (spec coverage)

- ReActLoop interleaved loop → **B1**. `process` re-scope → **B2**.
- Daemon chat wired with real tools + used `tool_runner` → **D1**.
- `ApprovalGate` trait + `TerminalApprovalGate` + RequireApproval consults gate → **A1/A2/A3**.
- exec auth (`.env`) + engine reuse (guarded runner) → **C1/C2**.
- CLI/TUI stay in body (no new crate) → no task moves them. ✓
- Reuse brain LLM types (no new abi types) → B1/B2 import `LlmResponse`/`StreamChunk`/`ToolDefinition`. ✓
- Defining acceptance test (file created; L2 y/n aborts) → **E2**.
- Multi-agent: disjoint file ownership (A=security, B=core, C=exec bin, D=daemon handler),
  dependency graph + batches above. ✓

## Notes for implementing agents

- **Do not invent APIs.** Where this plan says "verify the constructor / use the same
  pattern as X," read the referenced file first and match the real signature.
- **Commit per task.** Conflicts surface early; the coordinator can re-sync.
- **No new abi types**, **no new crate**, **do not move cli/ui out of body** — these are
  explicit Phase-1 constraints from the design.
- The old `Engine` (`impl/engine/`) is **left intact** this phase; its deletion is a
  separate later PR after this path is verified.
