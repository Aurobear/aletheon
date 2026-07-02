# M-A — Context Manager (Compaction Unify) — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Make the persisted multi-turn conversation path use the same tool-boundary-safe compaction the ReAct loop already uses, persist the compacted state across daemon restarts, and trigger compaction proactively before a turn's first LLM call — eliminating the long-conversation "报错/卡住/失忆" failure mode.

**Architecture:** Two compaction implementations exist today. The safe one (`AdvancedCompressor`, `find_tail_cut` + `prune_tool_outputs`) runs only in the ReAct loop. The naïve one (`SessionManager::compact_if_needed`, "keep last 6 non-system") governs the persisted history and can orphan a `tool_result` whose `tool_use` was summarized away → malformed provider request. This plan makes `SessionManager` delegate to `AdvancedCompressor`, fixes the recover path so compaction survives restart, and adds a pre-turn trigger.

**Tech Stack:** Rust, `tokio`, `rusqlite` (journal), `base::Message`, existing `AdvancedCompressor`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-A. Context Manager".

**Branch:** `auro/feat/20260701-aletheon-context-manager` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Fact | Anchor |
|---|---|
| Safe compactor: token-budget threshold, `find_tail_cut`, prunes tool outputs, iterative summary | `crates/runtime/src/impl/memory/compressor/mod.rs:38-89` (`maybe_compact`) |
| `find_tail_cut` aligns the cut off tool-message boundaries + keeps last user msg | `compressor/tail.rs:20-83` |
| Naïve path: keeps last 6 non-system, summarizes rest, **no tool-pair protection** | `daemon/session_manager.rs:113-236` (`compact_if_needed`) |
| `SessionManager.messages: Vec<Message>` (same type the compressor takes) | `session_manager.rs:19` |
| `force_compact` reuses `compact_if_needed` with threshold 0 | `session_manager.rs:249-259` |
| Post-turn compaction call site | `daemon/handler/chat.rs:674` (`sm.compact_if_needed(&*self.llm)`) |
| Seed of history into ReAct loop (no pre-turn compaction) | `chat.rs:459-467` |
| **Recover clears on `Compacted` and loses summary + tail** | `session_manager.rs:284-287` |
| `Compacted { before_count, after_count }` — no summary field | `session/journal.rs:38` |
| Journal recover = events after last `CheckpointBoundary` | `session/journal.rs:159-190` |
| `AdvancedCompressor` fields `previous_summary`/`context_window_tokens` are private | `compressor/mod.rs:17-19` |

---

## Design decisions (made for this plan)

1. **Unify by delegation, not by deleting the naïve code path's public API.**
   `SessionManager` keeps `compact_if_needed`/`force_compact` signatures (callers
   at `chat.rs:674` and elsewhere unchanged) but their *bodies* delegate to an
   owned `AdvancedCompressor`. Lowest blast radius.
2. **Hold one `AdvancedCompressor` per `SessionManager`** (a field) so its
   `previous_summary` enables iterative summaries across turns.
3. **Persist via the existing checkpoint mechanism, not a new snapshot file.**
   On successful compaction: write a `CheckpointBoundary`, then journal a new
   `Summary { text }` event and re-journal the surviving tail. Recover-from-last-
   checkpoint then reconstructs `[summary system message] ++ tail` exactly. This
   reuses `journal.rs`'s "events after last checkpoint" logic (`journal.rs:164`).
4. **Proactive trigger = compact the seeded history before the first LLM call**
   (`chat.rs`, just before line 459), in addition to the existing post-turn call.

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/impl/memory/compressor/mod.rs` | extract `compact_impl(force)`, add `force_compact`, add `last_summary()` getter |
| `crates/runtime/src/impl/session/journal.rs` | add `SessionEvent::Summary { text }` + event_type string |
| `crates/runtime/src/impl/daemon/session_manager.rs` | own `AdvancedCompressor`; delegate `compact_if_needed`/`force_compact`; journal boundary+summary+tail; fix `recover` |
| `crates/runtime/src/impl/daemon/handler/chat.rs` | pre-turn proactive compaction before seeding |

Each phase ends with build + commit. Default checks:
`cargo build -p runtime` and `cargo test -p runtime`.

---

## Phase 1 — Unify: SessionManager delegates to the safe compactor

### Task 1: Make `AdvancedCompressor` forceable + expose last summary

**Files:** Modify `crates/runtime/src/impl/memory/compressor/mod.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// compressor/mod.rs tests module (reuses SimpleLlm already defined there)
#[tokio::test]
async fn force_compact_ignores_threshold_and_exposes_summary() {
    // context window huge so the normal threshold is NOT exceeded
    let mut c = AdvancedCompressor::new(50, 200, 10_000_000);
    let llm = SimpleLlm;
    let mut messages = vec![Message::user("start")];
    for i in 0..8 {
        messages.push(Message::assistant(&format!("a{i} {}", "x".repeat(400))));
        messages.push(Message::user(&format!("u{i} {}", "y".repeat(400))));
    }
    // maybe_compact would be a no-op (under threshold)
    assert!(!c.maybe_compact(&mut messages.clone(), &llm).await.unwrap());
    // force_compact compacts anyway and records the summary
    let did = c.force_compact(&mut messages, &llm).await.unwrap();
    assert!(did, "force_compact should compact regardless of threshold");
    assert_eq!(c.last_summary(), Some("this is a summary"));
}
```

- [ ] **Step 2: Run — expected FAIL** (`force_compact`/`last_summary` undefined).

Run: `cargo test -p runtime compressor::tests::force_compact_ignores_threshold_and_exposes_summary`

- [ ] **Step 3: Refactor `maybe_compact` into `compact_impl(force)` + add methods**

Replace the body of `maybe_compact` (mod.rs:38-89) so it delegates, and add the
two new methods + getter:

```rust
/// Check if compaction is needed and perform it. Returns true if performed.
pub async fn maybe_compact<L: LlmProvider + ?Sized>(
    &mut self,
    messages: &mut Vec<Message>,
    llm: &L,
) -> Result<bool> {
    self.compact_impl(messages, llm, false).await
}

/// Compact regardless of the token threshold (still tool-boundary-safe).
pub async fn force_compact<L: LlmProvider + ?Sized>(
    &mut self,
    messages: &mut Vec<Message>,
    llm: &L,
) -> Result<bool> {
    self.compact_impl(messages, llm, true).await
}

/// The most recent summary produced by a compaction, if any.
pub fn last_summary(&self) -> Option<&str> {
    self.previous_summary.as_deref()
}

async fn compact_impl<L: LlmProvider + ?Sized>(
    &mut self,
    messages: &mut Vec<Message>,
    llm: &L,
    force: bool,
) -> Result<bool> {
    let total_tokens: usize = messages.iter().map(|m| m.estimate_tokens()).sum();
    if !force {
        let threshold = (self.context_window_tokens as f64 * 0.8) as usize;
        if total_tokens < threshold {
            return Ok(false);
        }
    }

    let cut = find_tail_cut(messages, &self.tail_config);
    if cut == 0 || cut >= messages.len() {
        return Ok(false);
    }
    let old_messages = &messages[..cut];
    let tail_messages = &messages[cut..];
    if old_messages.is_empty() {
        return Ok(false);
    }

    let mut pruned_messages = old_messages.to_vec();
    corpus::tools::tools::output::pruner::prune_tool_outputs(&mut pruned_messages, 0);

    let summary = self.generate_summary(&pruned_messages, llm).await?;

    let mut compacted = Vec::new();
    compacted.push(Message::system(format!(
        "{}\n{}\n[End Summary]",
        SUMMARY_PREFIX, summary
    )));
    compacted.extend_from_slice(tail_messages);

    self.previous_summary = Some(summary);

    let before = messages.len();
    *messages = compacted;
    info!(before, after = messages.len(), cut, "Context compacted (tail-protected)");
    Ok(true)
}
```

> This is a pure refactor of existing logic + a `force` bypass + a getter. The
> existing `test_compressor_actually_compacts` must still pass unchanged.

- [ ] **Step 4: Run — expected PASS.** Also: `cargo test -p runtime compressor`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/memory/compressor/mod.rs
git commit -m "feat(compressor): add force_compact + last_summary; extract compact_impl"
```

### Task 2: `SessionManager` owns and delegates to `AdvancedCompressor`

**Files:** Modify `crates/runtime/src/impl/daemon/session_manager.rs`.

- [ ] **Step 1: Write the failing test** (a summarized tail must never START with an orphan tool_result)

```rust
// session_manager.rs tests module (add if absent)
#[cfg(test)]
mod compaction_tests {
    use super::*;
    use base::message::{is_tool_message, Message};
    use cognit::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};
    use base::{ContentBlock, ToolDefinition};
    use async_trait::async_trait;

    struct StubLlm;
    #[async_trait]
    impl LlmProvider for StubLlm {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text { text: "SUMMARY".into() }],
                stop_reason: StopReason::EndTurn, usage: Usage::default(),
                cache_hit_tokens: 0, cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmStream> { unimplemented!() }
        fn name(&self) -> &str { "stub" }
        fn max_context_length(&self) -> usize { 1_000 }
    }

    #[tokio::test]
    async fn compaction_tail_never_starts_with_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        // small max_tokens so the threshold trips easily
        let mut sm = SessionManager::new(dir.path(), "s1".into(), 1_000).await.unwrap();
        // build a long history that interleaves tool_use/tool_result pairs
        for i in 0..12 {
            sm.push_assistant(&format!("assistant turn {i} {}", "x".repeat(400))).await;
            sm.push_raw(Message::tool_result(&format!("t{i}"), &"y".repeat(400), false)).await;
            sm.push_user(&format!("user {i} {}", "z".repeat(400))).await;
        }
        let did = sm.compact_if_needed(&StubLlm).await;
        assert!(did, "should compact");
        let hist = sm.history();
        // first non-system message after the summary must not be a bare tool_result
        let first_non_system = hist.iter().find(|m| !matches!(m.role, Role::System));
        if let Some(m) = first_non_system {
            assert!(!is_tool_message(m), "tail must not start with an orphan tool message");
        }
    }
}
```

> If `push_raw` (push an arbitrary `Message`) does not exist, add a minimal
> `pub async fn push_raw(&mut self, msg: Message)` that pushes to `self.messages`
> (journaling optional for tool messages, consistent with today's text-only journal).

- [ ] **Step 2: Run — expected FAIL** (old naïve split can start the tail with a tool_result; also `push_raw` may be missing).

Run: `cargo test -p runtime session_manager::compaction_tests::compaction_tail_never_starts_with_tool_result`

- [ ] **Step 3: Implement delegation**

Add the field + init, and replace `compact_if_needed`'s body:

```rust
// top of file
use crate::r#impl::memory::compressor::AdvancedCompressor;

pub struct SessionManager {
    pub session_id: String,
    messages: Vec<Message>,
    journal: EventJournal,
    max_tokens: usize,
    compaction_threshold: f64,
    compressor: AdvancedCompressor,   // NEW
}
```

```rust
// in new(), when building Self { .. }
compressor: AdvancedCompressor::new(
    (max_tokens as f64 * 0.25) as usize, // tail token budget
    4_000,                               // target summary chars
    max_tokens,                          // context window
),
```

```rust
/// Compact the context window if we exceed the threshold, using the
/// tool-boundary-safe compressor. Returns true if compaction happened.
pub async fn compact_if_needed(&mut self, llm: &dyn LlmProvider) -> bool {
    self.run_compaction(llm, false).await
}

pub async fn force_compact(&mut self, llm: &dyn LlmProvider) -> bool {
    if self.messages.len() <= 2 { return false; }
    self.run_compaction(llm, true).await
}

async fn run_compaction(&mut self, llm: &dyn LlmProvider, force: bool) -> bool {
    let before_count = self.messages.len();
    let did = if force {
        self.compressor.force_compact(&mut self.messages, llm).await
    } else {
        self.compressor.maybe_compact(&mut self.messages, llm).await
    }
    .unwrap_or(false);
    if !did { return false; }
    let after_count = self.messages.len();
    let summary = self.compressor.last_summary().unwrap_or("").to_string();
    self.persist_compaction(before_count, after_count, summary).await; // Phase 2 fills this in
    info!(before = before_count, after = after_count, "Context compaction complete");
    true
}
```

For Phase 1 only, provide a temporary `persist_compaction` that keeps today's
behavior (journal the `Compacted` marker); Phase 2 replaces it:

```rust
async fn persist_compaction(&mut self, before_count: usize, after_count: usize, _summary: String) {
    let _ = self.journal.append(SessionEvent::Compacted { before_count, after_count }).await;
}
```

Delete the old naïve body of `compact_if_needed` (session_manager.rs:113-236) and
the old `force_compact` (249-259); they are replaced by the three fns above.

- [ ] **Step 4: Run — expected PASS.** Full module: `cargo test -p runtime session_manager`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/daemon/session_manager.rs
git commit -m "feat(session): compact via tool-boundary-safe AdvancedCompressor (unify)"
```

---

## Phase 2 — Persist compacted state across restart

### Task 3: `Summary` journal event + checkpoint-based persistence + recover fix

**Files:** Modify `crates/runtime/src/impl/session/journal.rs` and
`crates/runtime/src/impl/daemon/session_manager.rs`.

- [ ] **Step 1: Write the failing test** (compaction survives a reopen)

```rust
// session_manager.rs compaction_tests module
#[tokio::test]
async fn compacted_history_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut sm = SessionManager::new(dir.path(), "s2".into(), 1_000).await.unwrap();
        for i in 0..12 {
            sm.push_assistant(&format!("assistant {i} {}", "x".repeat(400))).await;
            sm.push_user(&format!("user {i} {}", "z".repeat(400))).await;
        }
        assert!(sm.compact_if_needed(&StubLlm).await);
    }
    // Reopen: recover must include the summary system message, not an empty/regrown history
    let sm2 = SessionManager::new(dir.path(), "s2".into(), 1_000).await.unwrap();
    let hist = sm2.history();
    assert!(!hist.is_empty(), "recovered history must not be empty after compaction");
    assert!(
        hist.iter().any(|m| matches!(m.role, Role::System)
            && m.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text.contains("SUMMARY")))),
        "recovered history must contain the persisted summary"
    );
}
```

- [ ] **Step 2: Run — expected FAIL** (today `recover` clears on `Compacted` and drops the summary).

- [ ] **Step 3a: Add the `Summary` event to the journal**

```rust
// session/journal.rs — in enum SessionEvent
Summary {
    text: String,
},
```

```rust
// journal.rs — in the event_type match (around :108-113)
SessionEvent::Summary { .. } => "summary",
```

- [ ] **Step 3b: Persist boundary + summary + tail on compaction**

Replace the Phase-1 stub `persist_compaction` with:

```rust
async fn persist_compaction(&mut self, before_count: usize, after_count: usize, summary: String) {
    // marker (keeps existing observability), then a fresh checkpoint so recover
    // starts from the compacted state, then the summary + surviving tail.
    let _ = self.journal.append(SessionEvent::Compacted { before_count, after_count }).await;
    let iteration = self.turn_count();
    let _ = self.journal.append(SessionEvent::CheckpointBoundary { iteration }).await;
    if !summary.is_empty() {
        let _ = self.journal.append(SessionEvent::Summary { text: summary }).await;
    }
    // Re-journal the surviving tail (text content) after the checkpoint so a
    // reopen reconstructs [summary] ++ tail. System summary is emitted above.
    let tail: Vec<Message> = self.messages.iter()
        .filter(|m| !matches!(m.role, Role::System))
        .cloned()
        .collect();
    for m in &tail {
        let text: String = m.content.iter().filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()), _ => None
        }).collect::<Vec<_>>().join(" ");
        match m.role {
            Role::User => { let _ = self.journal.append(SessionEvent::UserMessage { content: text }).await; }
            Role::Assistant => { let _ = self.journal.append(SessionEvent::AssistantMessage { content: text }).await; }
            Role::System => {}
        }
    }
}
```

- [ ] **Step 3c: Fix `recover` to reconstruct the summary**

Replace the `Compacted` arm and add a `Summary` arm (session_manager.rs:276-290):

```rust
for event in &state.events_after_checkpoint {
    match event {
        SessionEvent::Summary { text } => {
            messages.push(Message::system(format!("[Conversation summary]\n{text}")));
        }
        SessionEvent::UserMessage { content } => messages.push(Message::user(content)),
        SessionEvent::AssistantMessage { content } => messages.push(Message::assistant(content)),
        SessionEvent::Compacted { .. } => { /* superseded by the checkpoint written right after */ }
        _ => {}
    }
}
```

> Because `persist_compaction` writes a fresh `CheckpointBoundary` *after* the
> `Compacted` marker, `recover` (which reads only events after the last
> checkpoint) sees `[Summary, ...tail]` and rebuilds the compacted state. The
> `Compacted` arm becomes a no-op for the pre-checkpoint marker.

- [ ] **Step 4: Run — expected PASS.** Also re-run Phase 1 tests to ensure no regression.

Run: `cargo test -p runtime session_manager`

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/session/journal.rs crates/runtime/src/impl/daemon/session_manager.rs
git commit -m "feat(session): persist compacted history across restart (Summary event + checkpoint)"
```

---

## Phase 3 — Proactive pre-turn compaction

### Task 4: Compact seeded history before the first LLM call

**Files:** Modify `crates/runtime/src/impl/daemon/handler/chat.rs`.

- [ ] **Step 1: Add the pre-turn trigger**

Immediately **before** reading `existing_messages` (chat.rs:459), compact the
persisted history so the seed handed to the ReAct loop is already within budget:

```rust
// chat.rs — just before `let existing_messages = { ... };` at :459
{
    let mut sm = self.session_manager.lock().await;
    let _ = sm.compact_if_needed(&*self.llm).await;
}
let existing_messages = {
    let sm = self.session_manager.lock().await;
    sm.history().to_vec()
};
```

- [ ] **Step 2: Build** `cargo build -p runtime` — expected: compiles.

- [ ] **Step 3: Manual smoke** (daemon running, small `max_tokens` in config):
  Drive a long multi-turn conversation past the threshold; confirm logs show
  "Context compacted (tail-protected)" *before* the turn's model call, no
  malformed-request/tool-pair errors, and that restarting the daemon mid-session
  resumes with the summary present (`memory`/history intact).

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/chat.rs
git commit -m "feat(chat): proactive pre-turn compaction on seeded history"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** unify (Task 1–2) ↔ M-A "reuse tool-boundary-safe compactor";
  persist (Task 3) ↔ M-A "persist the compacted history so it stops regrowing";
  proactive (Task 4) ↔ M-A "run compaction proactively before the turn's first LLM call".
- **Placeholder scan:** none — every step has real code + exact commands. The
  Phase-1 `persist_compaction` stub is explicitly replaced in Phase 3... (Task 3),
  not left as a TODO.
- **Type consistency:** `AdvancedCompressor::{maybe_compact,force_compact,last_summary}`
  signatures match Task 1; `SessionManager.messages: Vec<Message>` matches the
  compressor's `&mut Vec<Message>` (session_manager.rs:19); `SessionEvent::Summary`
  added in journal + handled in recover + emitted in persist.

## Risks / notes for the implementer

- **Hot path** — Phases 1–2 change how the persisted history is built. Keep the
  ReAct-loop compaction (`react_loop/step.rs:42,216`) untouched; it already uses
  `AdvancedCompressor` correctly. This plan only aligns the *session* path to it.
- **Journal is text-only today** — `push_user`/`push_assistant` store text; tool
  messages are not journaled. Persistence in Task 3 therefore reconstructs a
  text-only tail (consistent with current recover behavior). Full tool-structure
  persistence is out of scope (would need new event variants for tool blocks).
- **`prune_tool_outputs` path** — `corpus::tools::tools::output::pruner::prune_tool_outputs`
  is already used by the compressor; delegation inherits it. No new dependency.
- **`force_compact` semantics preserved** — old behavior (compact regardless of
  threshold, no-op when ≤2 messages) is retained via the guard + `force` bypass.
- **Backward compat** — old journals without `Summary`/second checkpoint still
  recover (the new `Summary` arm just never fires; the old `Compacted`-clear
  behavior is replaced by a no-op, which for a legacy pre-checkpoint `Compacted`
  means the pre-compaction events replay — acceptable, and self-heals on the next
  compaction). Note this in the PR description.
- **Does not resolve M-H** (the cognitive-memory bifurcation) — separate module.
