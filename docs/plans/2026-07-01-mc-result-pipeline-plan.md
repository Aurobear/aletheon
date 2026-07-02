# M-C — Result / Verification Pipeline — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Add an optional, pluggable verification seam between "the LLM produced a final answer" and "return it to the caller", so a configured `Verifier` can accept or reject-and-retry the answer — with a **no-op default that leaves current behavior byte-for-byte unchanged**.

**Architecture:** Doc 1 ("Result Pipeline") says the model's output is not the final answer; it should pass Runtime Verify → (execute) → Observation → … → Final Response. Today the ReAct loop returns the assistant text directly when there are no tool calls (`react_loop/step.rs:69-82`), with no verification hook. This plan introduces a `Verifier` trait in `base` (default `NoopVerifier`), holds an `Option<Arc<dyn Verifier>>` on `ReActLoop`, and calls it at the no-tool return site; on `Reject` it appends a revision request and re-loops, bounded by a small attempt cap.

**Tech Stack:** Rust, `async-trait` (already a `base` dep), `tokio`, `base::message::Message`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md § "M-C. Result / Verification pipeline"`

**Branch:** `auro/feat/20260701-aletheon-result-pipeline` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Fact | Anchor |
|---|---|
| Package names: `base` and `runtime` (bins `aletheond`, `aletheon-exec`) | `crates/base/Cargo.toml:2`, `crates/runtime/Cargo.toml:2,9,13` |
| No-tool final-answer return site (returns assistant text directly, no verify) | `crates/runtime/src/core/react_loop/step.rs:69-82` |
| Loop struct is `ReActLoop` (capital A), holds `config/messages/…` | `crates/runtime/src/core/react_loop/mod.rs:124` |
| `ReActLoop::new(config: RuntimeConfig)` builds all fields | `crates/runtime/src/core/react_loop/mod.rs:154-193` |
| `run<L,F,Fut>(&mut self, user_input, llm, tool_defs, execute_tool) -> Result<(String, TurnMetrics)>` | `crates/runtime/src/core/react_loop/step.rs:16-27` |
| `TurnMetrics { tool_calls_made, tool_errors, elapsed_ms, iterations, completed_normally }` | `crates/runtime/src/core/react_loop/mod.rs:30-36` |
| `run()` body is a `loop { ... }`; `continue` re-enters the LLM call | `crates/runtime/src/core/react_loop/step.rs` (loop around `:29-242`) |
| `base` already depends on `async-trait` and `tokio` (full) | `crates/base/Cargo.toml` `[dependencies]` |
| `base::policy` module exists (`pub mod execpolicy;`) — natural home for `verifier` | `crates/base/src/policy/mod.rs:1-3`, `crates/base/src/lib.rs:27,79` |
| Existing loop test pattern (scripted `LlmProvider`, `RuntimeConfig::default()`, `run()` with tool closure) to mirror | `crates/runtime/src/core/react_loop/mod.rs:542-585` |

---

## Design decisions (made for this plan)

1. **Trait lives in `base` (ABI), not `runtime`.** Verifiers may be implemented by
   any higher crate (`cognit` self-critique, a plugin) without depending on
   `runtime`. Mirrors how other cross-crate contracts live in `base`.
2. **No-op default = zero behavior change.** `ReActLoop.verifier` defaults to
   `None`. When `None`, the return site is unchanged. This is the safety guarantee.
3. **Reject → bounded retry, not hard fail.** On `Verdict::Reject { reason }` the
   loop records the rejected answer, appends a user-role revision request, and
   `continue`s. A per-turn attempt cap (`max_verify_attempts`, default 2) prevents
   infinite reject loops; once exhausted, the last answer is returned as-is.
4. **Seam only.** Concrete verifiers (self-critique, schema/goal checks) are
   explicit follow-ups; this plan ships the trait + no-op + wiring + one test verifier.

---

## File map

| File | Change |
|---|---|
| `crates/base/src/policy/verifier.rs` | **new** — `Verifier` trait, `Verdict` enum, `NoopVerifier` |
| `crates/base/src/policy/mod.rs` | add `pub mod verifier;` |
| `crates/base/src/lib.rs` | add `pub use policy::verifier;` (next to `pub use policy::execpolicy;` at `:79`) |
| `crates/runtime/src/core/react_loop/mod.rs` | add `verifier`/`verify_attempts`/`max_verify_attempts` fields + `set_verifier` |
| `crates/runtime/src/core/react_loop/step.rs` | call verifier at the no-tool return site (`:69-82`) |

Each phase ends with build + commit. Default checks: `cargo build -p base -p runtime` and `cargo test -p base`, `cargo test -p runtime react_loop`.

---

## Phase 1 — The `Verifier` trait in `base` (no-op default)

### Task 1: Add `Verifier` / `Verdict` / `NoopVerifier`

**Files:** Create `crates/base/src/policy/verifier.rs`; modify `crates/base/src/policy/mod.rs` and `crates/base/src/lib.rs`.

- [ ] **Step 1: Write the failing test** (put it in the new file's test module)

```rust
// crates/base/src/policy/verifier.rs  (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[tokio::test]
    async fn noop_verifier_always_accepts() {
        let v = NoopVerifier;
        let msgs = vec![Message::user("hi")];
        assert!(matches!(v.verify("any answer", &msgs).await, Verdict::Accept));
    }

    #[tokio::test]
    async fn reject_carries_reason() {
        struct Always;
        #[async_trait::async_trait]
        impl Verifier for Always {
            async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
                Verdict::Reject { reason: "nope".into() }
            }
        }
        match Always.verify("x", &[]).await {
            Verdict::Reject { reason } => assert_eq!(reason, "nope"),
            _ => panic!("expected reject"),
        }
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (module/types not defined).

Run: `cargo test -p base policy::verifier`

- [ ] **Step 3: Implement the trait + no-op**

```rust
// crates/base/src/policy/verifier.rs
//! Result-verification seam (M-C). A `Verifier` inspects a candidate final
//! answer and either accepts it or rejects it with a reason so the runtime can
//! request a revision. The default `NoopVerifier` always accepts, preserving
//! behavior when no verifier is configured.

use crate::message::Message;
use async_trait::async_trait;

/// Outcome of verifying a candidate final answer.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// The answer is acceptable; return it to the caller.
    Accept,
    /// The answer is rejected; `reason` is fed back to the model for a revision.
    Reject { reason: String },
}

/// Inspects a candidate final answer in the context of the conversation.
#[async_trait]
pub trait Verifier: Send + Sync {
    /// Verify the model's final text. `messages` is the full conversation so far
    /// (system + user + assistant + tool turns), for context-aware checks.
    async fn verify(&self, final_text: &str, messages: &[Message]) -> Verdict;
}

/// The default verifier: accepts everything (no behavior change).
pub struct NoopVerifier;

#[async_trait]
impl Verifier for NoopVerifier {
    async fn verify(&self, _final_text: &str, _messages: &[Message]) -> Verdict {
        Verdict::Accept
    }
}
```

```rust
// crates/base/src/policy/mod.rs
pub mod execpolicy;
pub mod verifier;
```

```rust
// crates/base/src/lib.rs — next to `pub use policy::execpolicy;` (:79)
pub use policy::verifier;
```

- [ ] **Step 4: Run — expected PASS.** `cargo test -p base policy::verifier`.

- [ ] **Step 5: Commit**

```bash
git add crates/base/src/policy/verifier.rs crates/base/src/policy/mod.rs crates/base/src/lib.rs
git commit -m "feat(base): add Verifier trait + NoopVerifier (result-pipeline seam)"
```

---

## Phase 2 — Wire the seam into the ReAct loop (default None)

### Task 2: Hold an optional verifier and call it at the return site

**Files:** Modify `crates/runtime/src/core/react_loop/mod.rs` and `crates/runtime/src/core/react_loop/step.rs`.

- [ ] **Step 1: Write the failing test** (add to the existing `tests` module in `mod.rs`, reusing the scripted-LLM pattern at `mod.rs:542-585`)

```rust
// crates/runtime/src/core/react_loop/mod.rs tests module
use base::policy::verifier::{Verdict, Verifier};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Rejects the first candidate answer, accepts all subsequent ones.
struct RejectOnce {
    seen: AtomicUsize,
}
#[async_trait]
impl Verifier for RejectOnce {
    async fn verify(&self, _text: &str, _msgs: &[base::message::Message]) -> Verdict {
        if self.seen.fetch_add(1, Ordering::SeqCst) == 0 {
            Verdict::Reject { reason: "first try rejected".into() }
        } else {
            Verdict::Accept
        }
    }
}

/// An LLM that always returns plain text (no tool calls), counting its calls.
struct TextLlm {
    calls: std::sync::Mutex<usize>,
}
#[async_trait]
impl LlmProvider for TextLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        Ok(LlmResponse {
            content: vec![ContentBlock::Text { text: format!("answer {n}") }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }
    async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmStream> {
        unimplemented!("not used in test")
    }
    fn name(&self) -> &str { "text" }
    fn max_context_length(&self) -> usize { 100_000 }
}

#[tokio::test]
async fn verifier_rejection_triggers_one_retry() {
    let cfg = RuntimeConfig {
        max_iterations: 5,
        session_id: "t".into(),
        learning_enabled: false,
        compaction_enabled: false,
        ..RuntimeConfig::default()
    };
    let mut lp = ReActLoop::new(cfg);
    lp.set_verifier(std::sync::Arc::new(RejectOnce { seen: AtomicUsize::new(0) }));
    let llm = TextLlm { calls: std::sync::Mutex::new(0) };
    let tool_defs: Vec<ToolDefinition> = vec![];
    let (out, _m) = lp
        .run("go", &llm, &tool_defs, |_id: &str, name: &str, _in: &serde_json::Value| {
            let name = name.to_string();
            async move { (format!("ran {name}"), false) }
        })
        .await
        .unwrap();
    // First answer rejected → loop retried → second answer accepted.
    assert_eq!(out, "answer 2", "rejected answer should be revised, got: {out}");
}

#[tokio::test]
async fn no_verifier_returns_first_answer_unchanged() {
    let cfg = RuntimeConfig { max_iterations: 5, session_id: "t".into(),
        learning_enabled: false, compaction_enabled: false, ..RuntimeConfig::default() };
    let mut lp = ReActLoop::new(cfg); // no set_verifier → None
    let llm = TextLlm { calls: std::sync::Mutex::new(0) };
    let tool_defs: Vec<ToolDefinition> = vec![];
    let (out, _m) = lp.run("go", &llm, &tool_defs,
        |_i: &str, n: &str, _in: &serde_json::Value| { let n = n.to_string(); async move { (n, false) } })
        .await.unwrap();
    assert_eq!(out, "answer 1", "no verifier = unchanged behavior");
}
```

- [ ] **Step 2: Run — expected FAIL** (`set_verifier` undefined; no retry logic).

Run: `cargo test -p runtime react_loop::tests::verifier_rejection_triggers_one_retry`

- [ ] **Step 3a: Add fields + setter** in `mod.rs`

```rust
// mod.rs — imports near the top of the module
use base::policy::verifier::{Verdict, Verifier};
use std::sync::Arc;
```

```rust
// mod.rs — ReActLoop struct (add after `reflection_engine`)
    /// Optional result verifier (M-C). None = no-op (unchanged behavior).
    verifier: Option<Arc<dyn Verifier>>,
    /// Verify attempts used this turn (reset at the start of run()).
    verify_attempts: usize,
    /// Max verify-reject retries per turn before returning as-is.
    max_verify_attempts: usize,
```

```rust
// mod.rs — in new(), inside Self { .. } (after reflection_engine)
    verifier: None,
    verify_attempts: 0,
    max_verify_attempts: 2,
```

```rust
// mod.rs — impl ReActLoop
/// Install a result verifier. Without this, verification is a no-op.
pub fn set_verifier(&mut self, verifier: Arc<dyn Verifier>) {
    self.verifier = Some(verifier);
}
```

- [ ] **Step 3b: Reset the per-turn counter** at the top of `run()` in `step.rs`

```rust
// step.rs — right after `let mut tool_errors: usize = 0;` (near :30)
self.verify_attempts = 0;
```

- [ ] **Step 3c: Call the verifier at the no-tool return site** (`step.rs:69-82`)

Replace the `if tool_calls.is_empty() { ... }` block body with:

```rust
if tool_calls.is_empty() {
    let final_text = text_parts.join("\n");

    // M-C: optional verification seam. Default (None) = unchanged behavior.
    if let Some(verifier) = self.verifier.clone() {
        if self.verify_attempts < self.max_verify_attempts {
            if let Verdict::Reject { reason } =
                verifier.verify(&final_text, &self.messages).await
            {
                self.verify_attempts += 1;
                // Record the rejected answer, then request a revision and re-loop.
                self.messages.push(Message::assistant(&final_text));
                self.messages.push(Message::user(&format!(
                    "[verification] Your previous answer was rejected: {reason}\n\
                     Please correct it and provide a better final answer."
                )));
                warn!(reason = reason.as_str(), "verifier rejected final answer; retrying");
                continue;
            }
        }
    }

    // Emit awareness: uncertainty from response + final response signal
    self.emit_thinking_complete("thinking", &final_text);
    self.emit_final_response("final_response");
    self.messages.push(Message::assistant(&final_text));
    let metrics = TurnMetrics {
        tool_calls_made,
        tool_errors,
        elapsed_ms: start.elapsed().as_millis() as u64,
        iterations: self.iteration,
        completed_normally: true,
    };
    return Ok((final_text, metrics));
}
```

> `continue` re-enters the `loop`, which re-runs the LLM call with the appended
> revision request. `max_iterations` still bounds the outer loop, and
> `max_verify_attempts` bounds reject-retries independently.

- [ ] **Step 4: Run — expected PASS.** `cargo test -p runtime react_loop` (both new tests + all existing loop tests must pass unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/react_loop/mod.rs crates/runtime/src/core/react_loop/step.rs
git commit -m "feat(react_loop): optional Verifier seam at final-answer return (default no-op)"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** trait + no-op default (Task 1) ↔ M-C "a `Verifier` trait (`base`)
  with a default no-op impl, so behavior is unchanged unless a verifier is configured";
  wiring at the return site (Task 2) ↔ M-C "Wire the hook at the `step.rs` return site".
- **Placeholder scan:** none — real trait, real wiring, real tests, exact commands.
- **Type consistency:** `Verifier::verify(&self, &str, &[Message]) -> Verdict` matches
  both the `base` unit test and the `runtime` call site; `ReActLoop` field types
  (`Option<Arc<dyn Verifier>>`) match `set_verifier`'s `Arc<dyn Verifier>`; return
  site rebuilds the exact `TurnMetrics` fields verified at `mod.rs:30-36`.

## Risks / notes for the implementer

- **No-op guarantee is load-bearing.** The default `verifier: None` path must be
  byte-identical to today. The `no_verifier_returns_first_answer_unchanged` test
  guards this — do not remove it.
- **Only the no-tool return site is wired.** The other `return Ok(...)` sites in
  `step.rs` (budget-exceeded `:112`, circuit-breaker `:128`, max-iteration fallbacks
  `:198-211`, `:242`) are *abnormal* exits — deliberately NOT verified (verifying a
  forced/aborted answer would loop pathologically). Document this in the PR.
- **Attempt cap prevents infinite reject loops.** With a stubborn verifier, after
  `max_verify_attempts` the last answer returns as-is (not an error). If a hard-fail
  policy is ever wanted, that is a follow-up config, not this seam.
- **Verifier latency is opt-in.** No verifier = no extra LLM/tool call. A concrete
  self-critique verifier (via `cognit`) would add a round-trip; keep it configurable.
- **Message shape on reject:** appends `assistant(final_text)` then `user(reason)` —
  valid provider ordering (assistant text → user), so no tool-pair breakage.
