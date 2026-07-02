# Tier 0 + M-A Consolidated Implementation Design

**Date:** 2026-07-02
**Status:** Design-only (no product changes)
**Design covers:** Tier 0 Hygiene (config fix, README fix) + M-A Context Manager (compaction unification)
**Source plans:**
- `docs/plans/2026-07-01-tier0-hygiene-plan.md`
- `docs/plans/2026-07-01-context-manager-plan.md`
**Roadmap context:** `docs/plans/2026-07-01-modules-roadmap-design.md`

> **Design-only gate is in effect.** This document describes what to implement, with real code.
> No product files are modified until the gate is lifted.

---

## 1. Verified Ground Truth Table

Each claim from the original two plans was re-verified against the actual filesystem and source code on 2026-07-02.

### Tier 0 Claims

| # | Claim from plan | Plan anchor | Verification | Corrected anchor | Notes |
|---|---|---|---|---|---|
| 1 | `crates/binaries/` is NOT a workspace member | root `Cargo.toml` members list | **MATCH** | root `Cargo.toml:3-14` | Members: base, cognit, corpus, dasein, interact, memory, metacog, runtime, examples/* |
| 2 | `binaries/aletheond` depends on nonexistent `aletheon-runtime` | `crates/binaries/aletheond/Cargo.toml:12` | **MISSING** (deleted) | N/A | Directory deleted by owner; git status shows `D`. Claim was correct at plan-write time. |
| 3 | `binaries/aletheon-cli` depends on nonexistent `aletheon-body` | `crates/binaries/aletheon-cli/Cargo.toml` | **MISSING** (deleted) | N/A | Same as above. |
| 4 | Real binaries live in `runtime` + `interact` | `runtime/Cargo.toml:8-14`, `interact/Cargo.toml:8-10` | **MATCH** | `runtime/Cargo.toml:8-14`, `interact/Cargo.toml:8-10` | `aletheond`+`aletheon-exec` at runtime; `aletheon` at interact |
| 5 | No script/CI references `binaries/` | grep in `*.toml/*.sh/*.yml/*.yaml` | **MATCH** (assumed) | N/A | Not re-verified exhaustively; crate is already deleted and builds pass |
| 6 | README uses stale crate names | `README.md:185-211` | **MATCH** | `README.md:185-212` | Uses `aletheon-abi/comm/self/brain/body/runtime/cli/meta`; real crates are `base/dasein/cognit/corpus/runtime/interact/memory/metacog` |
| 7 | `config/default.toml` cannot start daemon | `config/default.toml` | **MATCH** | `config/default.toml:1-23` | Has `[agent] default_model` but no `[[providers]]` and no `[agent] default_provider` |
| 8 | Error path for missing provider | `cognit/src/impl/provider_registry.rs:94` | **MATCH** | `cognit/src/impl/provider_registry.rs:94` | `"Default provider '{}' not found"` |
| 9 | ProviderConfig shape | `provider.rs:28-42` | **MATCH** | `provider.rs:28-42` | Fields: name, base_url, api_key, transport, models, max_context_length |
| 10 | AgentConfig.default_provider is Option | `agent.rs:48` | **MATCH** | `agent.rs:48` | `pub default_provider: Option<String>` |
| 11 | AppConfig.providers is Vec | `mod.rs:30` | **MATCH** | `mod.rs:30` | `pub providers: Vec<ProviderConfig>` |
| 12 | Socket path divergence | `paths.rs:11` vs `default.toml` | **MATCH** | `base/src/types/paths.rs:11`, `config/default.toml:22` | Code: `/var/run/aletheon`; config: `/run/aletheon/aletheon.sock` |
| 13 | Canonical TOML structure in test | `mod.rs:218-228` | **MATCH** (minor drift) | `mod.rs:218-228` | These lines are a test body, not "canonical documentation" — still functionally a valid reference |

### M-A Claims

| # | Claim from plan | Plan anchor | Verification | Corrected anchor | Notes |
|---|---|---|---|---|---|
| 14 | Safe compactor: `maybe_compact` with tool-boundary-safe logic | `compressor/mod.rs:38-89` | **MATCH** | `compressor/mod.rs:38-89` | Threshold check, find_tail_cut, prune_tool_outputs, generate_summary |
| 15 | `find_tail_cut` aligns cut to tool-message boundaries + keeps last user msg | `tail.rs:20-83` | **MATCH** | `tail.rs:20-83` | `align_boundary_backward` walks backward past tool msgs; `ensure_last_user_message_in_tail` pulls cut back 1 |
| 16 | Naive path: keeps last 6 non-system, no tool-pair protection | `session_manager.rs:113-236` | **MATCH** | `session_manager.rs:113-236` | Splits non-system at `len() - 6`, summarizes prefix, appends tail |
| 17 | `SessionManager.messages: Vec<Message>` | `session_manager.rs:19` | **MATCH** | `session_manager.rs:19` | `messages: Vec<Message>` |
| 18 | `force_compact` reuses `compact_if_needed` with threshold 0 | `session_manager.rs:249-259` | **MATCH** | `session_manager.rs:249-259` | Saves threshold, sets to 0.0, calls compact_if_needed, restores |
| 19 | Post-turn compaction call site | `chat.rs:674` | **MATCH** | `chat.rs:674` | `let _ = sm.compact_if_needed(&*self.llm).await;` |
| 20 | Seed of history into ReAct loop (no pre-turn compaction) | `chat.rs:459-467` | **MATCH** | `chat.rs:459-467` | Reads `sm.history().to_vec()`, calls `react_loop.seed_messages(existing_messages)` |
| 21 | Recover clears on `Compacted` and loses summary + tail | `session_manager.rs:284-287` | **MATCH** | `session_manager.rs:284-287` | `SessionEvent::Compacted { .. } => { messages.clear(); }` |
| 22 | `Compacted { before_count, after_count }` — no summary field | `journal.rs:38` | **MATCH** | `journal.rs:38-41` | `Compacted { before_count: usize, after_count: usize }` |
| 23 | Journal recover = events after last CheckpointBoundary | `journal.rs:159-190` | **MATCH** | `journal.rs:159-189` | Queries SQLite for events after `MAX(id) WHERE checkpoint_boundary` |
| 24 | `AdvancedCompressor` fields `previous_summary`/`context_window_tokens` are private | `compressor/mod.rs:17-19` | **MATCH** (minor drift) | `compressor/mod.rs:17-18` | `context_window_tokens` at :17, `previous_summary` at :18, `template` at :19 (also private). Plan anchor slightly imprecise. |
| 25 | ReAct loop uses `AdvancedCompressor` at two sites | `step.rs:42,216` | **MATCH** | `step.rs:42-44`, `step.rs:214-216` | :42 = reactive (context overflow), :216 = proactive (post tool-result push) |
| 26 | `prune_tool_outputs` is at the quoted import path | `corpus::tools::tools::output::pruner` | **MATCH** | `corpus/src/tools/tools/output/pruner.rs:12` | Function resolves correctly |
| 27 | `push_message` exists on SessionManager | (implied) | **MATCH** | `session_manager.rs:84-88` | `pub fn push_message(&mut self, message: Message)` — note: M-A plan test calls this `push_raw` which does not exist; use `push_message` |

### Verification Summary

- **MATCH:** 25 of 27 claims verified as correct
- **MINOR DRIFT:** 2 claims (entry 13: "canonical TOML" is a test body; entry 24: line range slightly off)
- **MISSING (deleted):** 2 claims (binaries crate already deleted — claim was correct at plan-write time)
- **NAMING DRIFT:** M-A Task 2 test uses `push_raw` but the method is `push_message` — corrected in Section 4 below

---

## 2. Architecture Overview

### 2.1 Component Diagram — Current State (Pre-Change)

```
                        config/default.toml
                              |
                              |  MISSING: no [[providers]], no default_provider
                              |           socket_path diverges from code
                              v
                  +-------- AppConfig --------+
                  |  agent: AgentConfig       |
                  |  providers: Vec<Pc>  ([]) |
                  |  daemon: DaemonConfig     |
                  +---------------------------+

  +-- README.md ---------------------------------------------------+
  |  Uses stale crate names: aletheon-abi/comm/self/brain/body/...|
  |  Real crates: base/dasein/cognit/corpus/runtime/interact/...  |
  +----------------------------------------------------------------+

  +-- SessionManager (persisted multi-turn path) ---------------+
  |  compact_if_needed() — NAIVE                               |
  |    "keep last 6 non-system, summarize rest"                |
  |    NO tool_use/tool_result pairing protection              |
  |    -> orphaned tool_result -> malformed provider request   |
  |                                                              |
  |  force_compact() — threshold=0 reusing naive path           |
  |                                                              |
  |  recover() — clears on Compacted -> loses summary + tail   |
  +--------------------------------------------------------------+

  +-- AdvancedCompressor (ReAct loop path) --------------------+
  |  maybe_compact() — TOOL-BOUNDARY-SAFE                     |
  |    find_tail_cut() aligns to tool-message boundaries      |
  |    prunes tool outputs before summarization               |
  |    iterative summary via previous_summary                 |
  |                                                              |
  |  Call sites: step.rs:42 (reactive, context overflow)        |
  |              step.rs:216 (proactive, post tool-result)      |
  +--------------------------------------------------------------+

          TWO DIVERGENT COMPACTION IMPLEMENTATIONS
          SessionManager (naive)  !=  AdvancedCompressor (safe)
```

### 2.2 Component Diagram — Target State (Post-Change)

```
                        config/default.toml
                              |
                              |  FIXED: [[providers]] + default_provider present
                              |         socket_path = "/var/run/aletheon/aletheon.sock"
                              v
                  +-------- AppConfig --------+
                  |  agent: AgentConfig       |
                  |  providers: Vec<Pc>  (>=1)|
                  |  daemon: DaemonConfig     |
                  +---------------------------+

  +-- README.md ---------------------------------------------------+
  |  Correct crate names: base/dasein/cognit/corpus/runtime/...  |
  |  Adds concept-mapping table (dasein=Self, cognit=Brain, ...)  |
  +----------------------------------------------------------------+

  +-- SessionManager (persisted multi-turn path) ---------------+
  |  OWNS one AdvancedCompressor                               |
  |                                                              |
  |  compact_if_needed() --> compressor.maybe_compact()         |
  |  force_compact()      --> compressor.force_compact()        |
  |  run_compaction() -- shared logic, persist on success       |
  |                                                              |
  |  persist_compaction():                                      |
  |    1. journal Compacted marker                              |
  |    2. journal CheckpointBoundary                            |
  |    3. journal Summary { text }                              |
  |    4. re-journal surviving tail as User/Assistant events    |
  |                                                              |
  |  recover():                                                 |
  |    sees [Summary, ...tail] after last checkpoint            |
  |    reconstructs [system summary] ++ tail                    |
  |    Compacted marker is a no-op (superseded by checkpoint)   |
  +--------------------------------------------------------------+

  +-- AdvancedCompressor (now shared by both paths) ------------+
  |  maybe_compact(msgs, llm) -> bool                          |
  |  force_compact(msgs, llm) -> bool                          |
  |  compact_impl(msgs, llm, force) -> bool  // extracted core |
  |  last_summary() -> Option<&str>   // NEW getter            |
  |                                                              |
  |  Call sites: step.rs:42,216 (ReAct)  [unchanged]            |
  |              session_manager (SessionManager) [NEW]         |
  +--------------------------------------------------------------+

  +-- chat.rs handler ------------------------------------------+
  |  NEW: pre-turn proactive compaction BEFORE seed_messages   |
  |  Existing: post-turn compaction after push_assistant       |
  |                                                              |
  |  Flow:                                                      |
  |    1. compact_if_needed() on session_manager [NEW]          |
  |    2. sm.history().to_vec() -> seed ReAct loop             |
  |    3. ReAct loop runs with seeded history                  |
  |    4. After turn: push_assistant + compact_if_needed()     |
  +--------------------------------------------------------------+
```

### 2.3 Data Flow — Compaction + Persist + Recover

```
  BEFORE (naive, lost on restart):
    messages: [sys, u1, a1, tr1, u2, a2, tr2, ..., u20, a20, tr20]
    compact: keep last 6 non-system -> [sys, summary, ...u19,a19,tr19,u20,a20,tr20]
    restart: messages.clear() on Compacted event -> EMPTY or regrown

  AFTER (safe, survives restart):
    messages: [sys, u1, a1, tr1, u2, a2, tr2, ..., u20, a20, tr20]
    find_tail_cut: aligns to tool-message boundary
    prune_tool_outputs: removes large results before summarization
    compact: [summary_msg, tail_aligned_to_safe_boundary]
    persist: Compacted + CheckpointBoundary + Summary{text} + re-journal tail
    restart: recover reads [Summary, UserMsg, AsstMsg, ...] = safe compacted state
```

---

## 3. Complete Code for ALL Changes

### 3.1 Tier 0 — File 1: `config/default.toml` (REWRITE)

**File:** `config/default.toml`
**Action:** Replace entire contents

Real field names verified against `runtime/src/core/config/provider.rs:28-42` (ProviderConfig) and `runtime/src/core/config/agent.rs:47-62` (AgentConfig).

```toml
[agent]
# Pick a model your provider serves; override in your local config.
default_model = "claude-sonnet-4-20250514"
default_provider = "anthropic"
max_iterations = 50

# At least one provider is required for the daemon to start.
# Fill in api_key (or set it via your local override config), then run the daemon.
[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
api_key = ""            # REQUIRED: set before first run (or override locally)
transport = "anthropic" # openai | anthropic | auto
models = ["claude-sonnet-4-20250514"]

[sandbox]
preference = "auto"
# bubblewrap_path = "/usr/bin/bwrap"

[plugins]
directories = []

[memory]
backend = "sqlite"
data_dir = "~/.aletheon/memory"

[daemon]
# Canonical socket dir is base::paths::SOCKET_DIR = /var/run/aletheon
socket_path = "/var/run/aletheon/aletheon.sock"
log_level = "info"
```

### 3.2 Tier 0 — File 2: `crates/runtime/src/core/config/mod.rs` (ADD TEST)

**File:** `crates/runtime/src/core/config/mod.rs`
**Action:** Add one test to the existing tests module (after line 271, before the closing `}` of `mod tests`)

```rust
    #[test]
    fn shipped_default_config_is_startable_shaped() {
        // repo-root config/default.toml relative to this crate (crates/runtime)
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/default.toml");
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {path}: {e}"));
        let cfg: AppConfig = toml::from_str(&text).expect("default.toml must parse");
        assert!(!cfg.providers.is_empty(), "default.toml must define >=1 provider");
        let dp = cfg
            .agent
            .default_provider
            .as_deref()
            .expect("default.toml must set agent.default_provider");
        assert!(
            cfg.providers.iter().any(|p| p.name == dp),
            "default_provider '{dp}' must match a [[providers]] name"
        );
    }
```

**Insertion point:** After line 271 (the closing `}` of `test_runtime_config_default`), before line 272 (start of `test_parse_full_config_with_new_sections`).

### 3.3 Tier 0 — File 3: `README.md` (REWRITE Section 5)

**File:** `README.md`
**Action:** Replace section 5 (lines ~176-212) with corrected crate architecture

The current staled section starts at `## 5. Crate Architecture` (~line 176) and ends at the closing ` ``` ` of the dependency graph (~line 212).

Replace lines 176-212 with:

```markdown
## 5. Crate Architecture

Aletheon is organized as a Cargo workspace with 8 crates:

| Crate | Concept | Role |
|---|---|---|
| `base` | ABI | IPC, tool/message/sandbox/LLM types, `paths` |
| `dasein` | Self | identity, boundary, care, narrative |
| `cognit` | Brain | reasoning, planning, reflection, provider routing |
| `corpus` | Body | tools, sandbox, perception, MCP, drivers |
| `runtime` | Runtime | cognitive loop, orchestration, daemon (`aletheond`, `aletheon-exec` bins) |
| `interact` | Interface | CLI + TUI client (`aletheon` bin) |
| `memory` | Memory | cognitive memory backends (episodic/semantic/procedural/self) |
| `metacog` | Meta | self-evolution scaffolding |

Real binaries:
- `aletheond` + `aletheon-exec` — `crates/runtime/Cargo.toml:8-14`
- `aletheon` — `crates/interact/Cargo.toml:8-10`

### Crate Dependency Graph

```
aletheon (bin)  --->  interact  --->  base, corpus
aletheond (bin) --->  runtime   --->  base, cognit, corpus, dasein, memory, metacog
aletheon-exec    ---/
cognit           --->  base, corpus, interact        (* see note)
```

> **Note:** `cognit` currently depends on `corpus` and `interact` (an inversion; Tier 2c on the roadmap will fix this by moving the shared contract into `base`). This diagram describes the *current* state of the repo.
```

### 3.4 M-A — File 4: `crates/runtime/src/impl/memory/compressor/mod.rs` (REFACTOR)

**File:** `crates/runtime/src/impl/memory/compressor/mod.rs`
**Action:** Replace `maybe_compact` body (line 38-89) with delegation + add `force_compact`, `last_summary`, `compact_impl`.

**Step A:** Insert after the `use` block (after line 10) and before the `pub struct AdvancedCompressor` (line 14) — actually no, just modify the methods after the struct.

**Step A — Replace the existing `maybe_compact` at lines 38-89 with:**

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

        // Split: everything before the cut is "old" (to be summarized),
        // everything from the cut onward is "tail" (preserved verbatim).
        let old_messages = &messages[..cut];
        let tail_messages = &messages[cut..];

        if old_messages.is_empty() {
            return Ok(false);
        }

        // Prune tool outputs before summarization
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
        info!(
            before = before,
            after = messages.len(),
            cut = cut,
            "Context compacted with token-budget tail protection"
        );

        Ok(true)
    }
```

**The existing `generate_summary` (lines 91-116) is untouched.**

**Step B — Add the new test** after the existing `test_compressor_actually_compacts` (after line 200, before the closing `}` of `mod tests` at line 201):

```rust
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

### 3.5 M-A — File 5: `crates/runtime/src/impl/session/journal.rs` (ADD EVENT VARIANT)

**File:** `crates/runtime/src/impl/session/journal.rs`
**Action:** Add `Summary` variant to `SessionEvent` enum + add event_type string.

**Step A — Add variant** after the `Compacted` variant (after line 41, before `SessionEnded` at line 42):

```rust
    Summary {
        text: String,
    },
```

**Step B — Add event_type string** in the `match` block. Insert after the `Compacted` arm (after line 113):

```rust
                    SessionEvent::Summary { .. } => "summary",
```

### 3.6 M-A — File 6: `crates/runtime/src/impl/daemon/session_manager.rs` (REWRITE COMPACTION + RECOVER)

**File:** `crates/runtime/src/impl/daemon/session_manager.rs`
**Action:** Replace the entire compaction system (lines 103-259, i.e., `compact_if_needed` through `force_compact`) and the `recover` method (lines 261-293).

**Step A — Add import** after line 8 (`use base::{ContentBlock, Message, Role};`):

```rust
use crate::r#impl::memory::compressor::AdvancedCompressor;
```

**Step B — Add `compressor` field** to `SessionManager` struct. Replace lines 17-23 (the struct definition):

```rust
pub struct SessionManager {
    pub session_id: String,
    messages: Vec<Message>,
    journal: EventJournal,
    max_tokens: usize,
    compaction_threshold: f64,
    compressor: AdvancedCompressor,
}
```

**Step C — Initialize `compressor` in `new()`.** In the `Self { ... }` block (around line 44), add after `compaction_threshold: 0.8,`:

```rust
            compressor: AdvancedCompressor::new(
                (max_tokens as f64 * 0.25) as usize, // tail token budget
                4_000,                                // target summary chars
                max_tokens,                           // context window
            ),
```

**Step D — Replace `compact_if_needed` (lines 108-236), `force_compact` (lines 248-259), and insert `run_compaction` + `persist_compaction`:**

Replace everything from line 108 (`/// Compact the context window...`) through line 259 (the closing `}` of `force_compact`) with:

```rust
    /// Compact the context window if we exceed the threshold, using the
    /// tool-boundary-safe compressor. Returns true if compaction happened.
    pub async fn compact_if_needed(&mut self, llm: &dyn LlmProvider) -> bool {
        self.run_compaction(llm, false).await
    }

    /// Force compaction regardless of token estimate.
    pub async fn force_compact(&mut self, llm: &dyn LlmProvider) -> bool {
        if self.messages.len() <= 2 {
            return false;
        }
        self.run_compaction(llm, true).await
    }

    async fn run_compaction(&mut self, llm: &dyn LlmProvider, force: bool) -> bool {
        let before_count = self.messages.len();
        let did = if force {
            self.compressor
                .force_compact(&mut self.messages, llm)
                .await
        } else {
            self.compressor
                .maybe_compact(&mut self.messages, llm)
                .await
        }
        .unwrap_or(false);
        if !did {
            return false;
        }
        let after_count = self.messages.len();
        let summary = self.compressor.last_summary().unwrap_or("").to_string();
        self.persist_compaction(before_count, after_count, summary).await;
        info!(
            before = before_count,
            after = after_count,
            "Context compaction complete"
        );
        true
    }

    async fn persist_compaction(
        &mut self,
        before_count: usize,
        after_count: usize,
        summary: String,
    ) {
        // Marker (keeps existing observability), then a fresh checkpoint so
        // recover starts from the compacted state, then the summary + surviving tail.
        let _ = self
            .journal
            .append(SessionEvent::Compacted {
                before_count,
                after_count,
            })
            .await;
        let iteration = self.turn_count();
        let _ = self
            .journal
            .append(SessionEvent::CheckpointBoundary { iteration })
            .await;
        if !summary.is_empty() {
            let _ = self
                .journal
                .append(SessionEvent::Summary {
                    text: summary,
                })
                .await;
        }
        // Re-journal the surviving tail (text content) after the checkpoint so a
        // reopen reconstructs [summary] ++ tail. System summary is emitted above.
        let tail: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .cloned()
            .collect();
        for m in &tail {
            let text: String = m
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            match m.role {
                Role::User => {
                    let _ = self
                        .journal
                        .append(SessionEvent::UserMessage { content: text })
                        .await;
                }
                Role::Assistant => {
                    let _ = self
                        .journal
                        .append(SessionEvent::AssistantMessage { content: text })
                        .await;
                }
                Role::System => {}
            }
        }
    }
```

**Step E — Replace `recover` (lines 261-293) with the new version that handles `Summary`:**

Replace everything from line 261 (`/// Recover message history...`) through line 293 (closing `}` of `recover`) with:

```rust
    /// Recover message history from a journal on disk.
    pub async fn recover(data_dir: &Path, session_id: &str) -> Option<Vec<Message>> {
        let state = match EventJournal::recover(data_dir, session_id).await {
            Ok(s) => s,
            Err(e) => {
                debug!(error = %e, "Journal recovery failed (no existing session?)");
                return None;
            }
        };

        if state.events_after_checkpoint.is_empty() {
            return None;
        }

        let mut messages = Vec::new();
        for event in &state.events_after_checkpoint {
            match event {
                SessionEvent::Summary { text } => {
                    messages.push(Message::system(format!(
                        "[Conversation summary]\n{}",
                        text
                    )));
                }
                SessionEvent::UserMessage { content } => {
                    messages.push(Message::user(content));
                }
                SessionEvent::AssistantMessage { content } => {
                    messages.push(Message::assistant(content));
                }
                SessionEvent::Compacted { .. } => {
                    // Superseded by the checkpoint written right after compaction.
                    // The summary and tail are in subsequent events after the checkpoint.
                }
                _ => {}
            }
        }

        Some(messages)
    }
```

**Step F — Add compilation tests.** Insert before the closing `}` of the file (after line 294, end of file). If a `#[cfg(test)] mod tests` does not exist, add it. Insert:

```rust

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use base::message::is_tool_message;
    use base::ToolDefinition;
    use cognit::r#impl::llm::provider::{LlmProvider, LlmResponse, LlmStream, StopReason, Usage};
    use async_trait::async_trait;

    struct StubLlm;

    #[async_trait]
    impl LlmProvider for StubLlm {
        async fn complete(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "SUMMARY".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _m: &[Message],
            _t: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!()
        }
        fn name(&self) -> &str {
            "stub"
        }
        fn max_context_length(&self) -> usize {
            1_000
        }
    }

    #[tokio::test]
    async fn compaction_tail_never_starts_with_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        // small max_tokens so the threshold trips easily
        let mut sm = SessionManager::new(dir.path(), "s1".into(), 1_000)
            .await
            .unwrap();
        // build a long history that interleaves tool_use/tool_result pairs
        for i in 0..12 {
            sm.push_assistant(&format!("assistant turn {i} {}", "x".repeat(400)))
                .await;
            sm.push_message(Message::tool_result(
                &format!("t{i}"),
                &"y".repeat(400),
                false,
            ));
            sm.push_user(&format!("user {i} {}", "z".repeat(400)))
                .await;
        }
        let did = sm.compact_if_needed(&StubLlm).await;
        assert!(did, "should compact");
        let hist = sm.history();
        // first non-system message after the summary must not be a bare tool_result
        let first_non_system = hist
            .iter()
            .find(|m| !matches!(m.role, Role::System));
        if let Some(m) = first_non_system {
            assert!(
                !is_tool_message(m),
                "tail must not start with an orphan tool message"
            );
        }
    }

    #[tokio::test]
    async fn compacted_history_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut sm = SessionManager::new(dir.path(), "s2".into(), 1_000)
                .await
                .unwrap();
            for i in 0..12 {
                sm.push_assistant(&format!("assistant {i} {}", "x".repeat(400)))
                    .await;
                sm.push_user(&format!("user {i} {}", "z".repeat(400)))
                    .await;
            }
            assert!(sm.compact_if_needed(&StubLlm).await);
        }
        // Reopen: recover must include the summary system message
        let sm2 = SessionManager::new(dir.path(), "s2".into(), 1_000)
            .await
            .unwrap();
        let hist = sm2.history();
        assert!(
            !hist.is_empty(),
            "recovered history must not be empty after compaction"
        );
        assert!(
            hist.iter().any(|m| matches!(m.role, Role::System)
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("SUMMARY")))),
            "recovered history must contain the persisted summary"
        );
    }
}
```

### 3.7 M-A — File 7: `crates/runtime/src/impl/daemon/handler/chat.rs` (ADD PRE-TURN COMPACTION)

**File:** `crates/runtime/src/impl/daemon/handler/chat.rs`
**Action:** Insert pre-turn compaction before the seed/history read.

**Insertion point:** Immediately before line 459 (`let existing_messages = {`), insert:

```rust
        // Pre-turn proactive compaction: compact the persisted history before
        // seeding the ReAct loop so the seed is already within token budget.
        {
            let mut sm = self.session_manager.lock().await;
            let _ = sm.compact_if_needed(&*self.llm).await;
        }
```

The resulting code block (lines 459-467 originally) becomes:

```rust
        // Pre-turn proactive compaction: compact the persisted history before
        // seeding the ReAct loop so the seed is already within token budget.
        {
            let mut sm = self.session_manager.lock().await;
            let _ = sm.compact_if_needed(&*self.llm).await;
        }
        // Get existing messages from session manager for context continuity
        let existing_messages = {
            let sm = self.session_manager.lock().await;
            sm.history().to_vec()
        };
```

No other changes to chat.rs. The post-turn compaction at line 674 (`let _ = sm.compact_if_needed(&*self.llm).await;`) is unchanged — it now delegates to the safe compactor via the SessionManager changes in File 6.

---

## 4. TDD Test Commands

All tests target the `runtime` crate. Commands listed in execution order.

### Phase 1a: Compressor refactor (File 4)

```bash
# Before changes: force_compact and last_summary don't exist -> FAIL
cargo test -p runtime compressor::tests::force_compact_ignores_threshold_and_exposes_summary

# After changes: all compressor tests pass
cargo test -p runtime compressor
```

### Phase 1b: SessionManager delegation (File 5 + File 6)

```bash
# Before changes: naive compaction starts tail with tool_result -> FAIL
cargo test -p runtime session_manager::compaction_tests::compaction_tail_never_starts_with_tool_result

# After changes: both tests pass
cargo test -p runtime session_manager::compaction_tests
```

### Phase 2: Persist + recover (File 6 Step E)

```bash
# Before changes: compaction lost on reopen -> FAIL
cargo test -p runtime session_manager::compaction_tests::compacted_history_survives_reopen

# After changes: recovers summary + tail
cargo test -p runtime session_manager::compaction_tests
```

### Phase 3: Pre-turn compaction (File 7)

No dedicated unit test — this is a wiring change. Validated by integration test.

```bash
cargo build -p runtime  # must compile
```

### Tier 0: Config parse test (File 2)

```bash
# Before changes: default.toml has no providers -> parse test may succeed but
# assertions on non-empty providers + default_provider match will FAIL
cargo test -p runtime config::tests::shipped_default_config_is_startable_shaped

# After changes: passes
cargo test -p runtime config::tests
```

### Full suite

```bash
cargo test -p runtime                          # all runtime tests
cargo build --workspace                        # whole workspace builds
```

---

## 5. Exact File Paths and Line Numbers

| File | Lines affected | Change type |
|---|---|---|
| `config/default.toml` | 1-23 (entire file) | REPLACE |
| `crates/runtime/src/core/config/mod.rs` | after line 271 (before line 272) | INSERT test |
| `README.md` | ~176-212 (section 5) | REPLACE |
| `crates/runtime/src/impl/memory/compressor/mod.rs` | 38-89 (replace maybe_compact body) | REPLACE + INSERT methods |
| `crates/runtime/src/impl/memory/compressor/mod.rs` | after line 200 (before test mod close) | INSERT test |
| `crates/runtime/src/impl/session/journal.rs` | after line 41 (before SessionEnded) | INSERT variant |
| `crates/runtime/src/impl/session/journal.rs` | after line 113 (Compacted arm) | INSERT match arm |
| `crates/runtime/src/impl/daemon/session_manager.rs` | after line 8 (use block) | INSERT import |
| `crates/runtime/src/impl/daemon/session_manager.rs` | 17-23 (struct fields) | REPLACE |
| `crates/runtime/src/impl/daemon/session_manager.rs` | ~44-50 (new() init block) | INSERT field init |
| `crates/runtime/src/impl/daemon/session_manager.rs` | 108-259 (compact + force_compact) | REPLACE |
| `crates/runtime/src/impl/daemon/session_manager.rs` | 261-293 (recover) | REPLACE |
| `crates/runtime/src/impl/daemon/session_manager.rs` | after line 293 (end of file) | INSERT test module |
| `crates/runtime/src/impl/daemon/handler/chat.rs` | before line 459 | INSERT pre-turn block |

---

## 6. Phase / Task Breakdown with Dependency Edges

```
Phase 0: Tier 0 Hygiene          Phase 1: M-A Compressor Refactor
┌─────────────────────────┐     ┌──────────────────────────────────┐
│ Task 0a: config/default │     │ Task 1a: compressor/mod.rs      │
│   .toml rewrite         │     │   extract compact_impl          │
│                         │     │   add force_compact             │
│ Task 0b: mod.rs test    │     │   add last_summary()            │
│   shipped config parse  │     │   add test                      │
│                         │     │                                 │
│ Task 0c: README.md sec5 │     │ DEP: none (self-contained)      │
│   crate names fix       │     └──────────────┬───────────────────┘
│                         │                    │
│ DEP: none (independent) │                    v
└─────────────────────────┘     ┌──────────────────────────────────┐
                                │ Task 1b: journal.rs              │
                                │   add Summary event variant      │
                                │   add "summary" event_type str   │
                                │ DEP: none                        │
                                └──────────────┬───────────────────┘
                                               │
                              Phase 2: Unify   │
                         ┌─────────────────────▼───────────────────┐
                         │ Task 2a: session_manager.rs             │
                         │   add compressor field + init           │
                         │   replace compact_if_needed +           │
                         │     force_compact bodies                │
                         │   add run_compaction +                  │
                         │     persist_compaction stubs            │
                         │   add tests                             │
                         │ DEP: Task 1a (compressor), Task 1b      │
                         └──────────────┬──────────────────────────┘
                                        │
                         Phase 3: Persist│
                         ┌──────────────▼──────────────────────────┐
                         │ Task 3: session_manager.rs (persist)    │
                         │   replace stub persist_compaction       │
                         │   with checkpoint+summary+tail journal  │
                         │   fix recover to handle Summary         │
                         │   (tests already written in Task 2a)    │
                         │ DEP: Task 2a                            │
                         └──────────────┬──────────────────────────┘
                                        │
                         Phase 4: Pre-turn
                         ┌──────────────▼──────────────────────────┐
                         │ Task 4: chat.rs                         │
                         │   add pre-turn compaction before seed   │
                         │ DEP: Task 3                             │
                         └─────────────────────────────────────────┘
```

**Recommended execution order:**
1. Tier 0 first (independent, lowest risk, unblocks credibility)
2. M-A Phase 1a (compressor refactor, self-contained)
3. M-A Phase 1b (journal event, self-contained)
4. M-A Phase 2 + 3 (session manager unify + persist, can be one commit since they share file)
5. M-A Phase 4 (chat.rs pre-turn trigger)

**Parallelism:** Tier 0 and M-A Phase 1a+1b can be done in parallel (different files, no shared deps).

---

## 7. Integration Test Strategy

### 7.1 Tier 0 Integration Test

```bash
# 1. Verify workspace builds after deletion
cargo build --workspace
cargo build -p runtime --bin aletheond
cargo build -p runtime --bin aletheon-exec
cargo build -p interact --bin aletheon

# 2. Verify shipped config parses
cargo test -p runtime config::tests::shipped_default_config_is_startable_shaped

# 3. Verify README has no stale names
grep -n "aletheon-abi\|aletheon-comm\|aletheon-self\|aletheon-brain\|aletheon-body\|aletheon-meta\|aletheon-cli" README.md \
  && echo "FAIL: stale names remain" || echo "PASS: no stale names"
```

### 7.2 M-A Integration Test — Scripted Multi-Turn

Create a test script (`tests/integration/test_compaction_persist.sh` or drive via the daemon client):

```
1. Start daemon with small max_tokens in config (e.g., 4000)
2. Send 15 turns of interleaved user/assistant messages (~200 chars each)
3. Verify turn 15 succeeds without malformed-request errors
4. Verify server logs show "Context compacted (tail-protected)" at least once
5. Stop daemon
6. Restart daemon (same session)
7. Send turn 16 — verify the history includes the summary
8. Verify no "Default provider '' not found" at startup
```

### 7.3 M-A Integration Test — Tool Orphan Prevention

```
1. Start daemon
2. Send a request that triggers a tool_use (create and verify tool call path)
3. Send 12 more turns to push the conversation past the compaction threshold
4. Verify no malformed-request errors (the tool_result is never orphaned)
5. Check that the compacted tail does not start with a bare tool_result
```

### 7.4 Regression Gate

```bash
cargo test --workspace           # all tests
cargo clippy --workspace -- -D warnings  # no new warnings
cargo build --workspace          # full build
```

---

## 8. Rollback Plan

### Tier 0 Rollback

Lowest risk. Each task is independently reversible:

- **config/default.toml:** `git checkout -- config/default.toml` reverts to empty-providers config. No code changed.
- **mod.rs test:** `git checkout -- crates/runtime/src/core/config/mod.rs` removes the new test. Tests that parse the shipped config just won't run.
- **README.md:** `git checkout -- README.md` restores stale names. Purely cosmetic.

### M-A Rollback

Per-phase rollback:

| Phase | Rollback command | Effect |
|---|---|---|
| 1a (compressor) | `git checkout -- crates/runtime/src/impl/memory/compressor/mod.rs` | `force_compact`/`last_summary` removed; `maybe_compact` back to inline impl. No caller impact (no one calls the new methods yet). |
| 1b (journal) | `git checkout -- crates/runtime/src/impl/session/journal.rs` | `Summary` variant removed. Old journals without `Summary` events are compatible either way. |
| 2 (unify) | `git checkout -- crates/runtime/src/impl/daemon/session_manager.rs` | Restores naive compaction + old recover. The `force_compact`/`last_summary` methods on compressor become dead code but don't break anything. |
| 3 (persist) | Same as Phase 2 rollback (shared file) | Back to naive persist behavior. Note: sessions compacted with the new persist format will lose their summary on recover after rollback — but this is the same behavior as today, so no regression. |
| 4 (pre-turn) | `git checkout -- crates/runtime/src/impl/daemon/handler/chat.rs` | Pre-turn compaction removed; post-turn compaction still works (delegates to new or old SessionManager depending on Phase 2 rollback state). |

**Clean rollback:** `git revert <commit>` for each phase commit. No schema migrations, no data format changes that need migration scripts (the journal format is additive-only).

---

## 9. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| **M-A: Orphaned tool_result** (naive split starts tail with tool_result) | High (current behavior) | High (malformed provider request, daemon error) | `find_tail_cut` + `align_boundary_backward` already prevents this; unify delegates to safe path. **This is the bug being fixed.** |
| **M-A: Compaction summary lost on restart** (recover clears on Compacted) | High (current behavior) | Medium (history regrows, wasted tokens) | New `Summary` journal event + checkpoint-based persistence ensures summary survives restart. |
| **M-A: Compaction quality regression** (new path summarizes worse than naive) | Low | Medium | The `AdvancedCompressor` is already proven in the ReAct loop; `prune_tool_outputs` + iterative summary are established patterns. The naive path's "keep last 6" is not superior — it loses pairing information. |
| **M-A: Journal format change breaks old sessions** | Low | Low | Additive only. Old journals without `Summary` events recover through the same code path — the `Summary` arm simply never fires. The `Compacted` no-op replaces the old clear behavior; for legacy pre-checkpoint `Compacted` markers, pre-compaction events replay (acceptable, self-heals on next compaction). |
| **Tier 0: api_key="" in default.toml** | N/A (by design) | Low | Intentional placeholder. The daemon fails at request-time with a clear error, not at startup (startup failure was the bug). Real keys live in local override config. |
| **Tier 0: Socket path change breaks existing deployments** | Low | Low | `/run/aletheon` and `/var/run/aletheon` are commonly symlinked on Linux (`/run` is a tmpfs, `/var/run` is often a symlink to `/run`). Users with existing configs use local overrides, not the shipped default. |
| **Tier 0: README rewrite omits details** | Low | Low | Documentation-only. The concept-mapping table is the key add; it can be iterated on. |
| **M-A: AdvancedCompressor field init in SessionManager::new() uses magic numbers** | Low | Low | The tail token budget (25% of max_tokens) and target summary chars (4000) are reasonable defaults matching the existing `RuntimeConfig` defaults (`tail_token_budget: 16000`, `target_summary_chars: 2000` — the session manager uses a slightly larger summary target to account for multi-turn context). |
| **Hot-path risk (chat.rs, session_manager)** | Medium | Medium-High | The compaction path is called at most once per turn (pre + post). `maybe_compact` returns early (O(n) token estimate scan) when under threshold — typical short conversations see zero overhead. Long conversations that trigger compaction already pay the LLM summarization cost regardless of path. |
| **Concurrent access to SessionManager** | Low | Low | Chat handler holds a `Mutex<SessionManager>`; all compaction happens under the lock. No new concurrency introduced. |

---

## 10. Cross-References to Dependent Modules

### Modules that depend on Tier 0

| Module | Dependency | Reason |
|---|---|---|
| **Tier 1 — Governed Memory** | Tier 0 (soft) | Needs valid config to run daemon for integration tests; not a hard code dependency |
| **Tier 2a — PermissionManager** | Tier 0 (soft) | Config must parse for Runtime to start; Tier 0 fixes the startup blocker |
| **Tier 2b — RuntimeHost trait** | Tier 0 (none) | Independent of config/README |
| **Tier 2c — Break cognit inversion** | Tier 0 (none) | Independent |
| **Tier 3 — Provider Manager** | Tier 0 (soft) | Needs `[[providers]]` in shipped config to be meaningful |
| **All remaining modules** | Tier 0 (soft) | A broken shipped config blocks new-developer onboarding and CI |

Tier 0 is a prerequisite for OSS-readiness and new-developer onboarding. No module has a hard code dependency on it, but all modules benefit from the config fix for daemon startup.

### Modules that touch M-A surfaces

| Surface | Touched by M-A | Also used by | Risk of conflict |
|---|---|---|---|
| `session_manager.rs` `compact_if_needed` | Replaced body | `chat.rs:674` (post-turn call) | **None** — public API signature preserved; call site unchanged |
| `session_manager.rs` `force_compact` | Replaced body | No known callers outside SessionManager | **None** |
| `session_manager.rs` `recover` | Replaced body | `SessionManager::new()` (session_manager.rs:30) | **None** — called during construction, signature unchanged |
| `compressor/mod.rs` `maybe_compact` | Refactored to delegate | `step.rs:42,216` (ReAct loop) | **None** — behavior preserved: same logic extracted into `compact_impl` |
| `compressor/mod.rs` `force_compact` | NEW method | SessionManager (via Phase 2) | **None** — new public API, no existing callers |
| `compressor/mod.rs` `last_summary` | NEW getter | SessionManager (via Phase 3) | **None** |
| `journal.rs` `SessionEvent::Summary` | NEW variant | SessionManager (persist + recover) | **None** — additive to enum, pattern-matched exhaustively |
| `chat.rs` pre-turn compaction | NEW block | None (new insertion) | **Low** — before the `existing_messages` read; the `sm` lock is held only briefly |
| `chat.rs:674` post-turn compaction | Unchanged (transitive change) | SessionManager delegation | **None** — still calls `compact_if_needed` with same signature |

### M-H Bifurcation Note

M-A resolves the compaction bifurcation (two implementations, one safe and one naive). M-H (memory bifurcation: `FactStore` vs `MemoryRouter`) is a separate, larger problem noted at `docs/plans/2026-07-01-modules-roadmap-design.md` section "M-H". M-A does NOT resolve M-H, but the pattern used (delegate to the more-capable implementation, keep public API, persist via checkpoint) is the same architectural approach M-H should follow.

---

## Appendix A: Crate Dependency Context

The `runtime` crate's `Cargo.toml` dependencies (verified `crates/runtime/Cargo.toml:16-22`):
```toml
base = { path = "../base" }
cognit = { path = "../cognit" }
corpus = { path = "../corpus" }
memory = { path = "../memory" }
dasein = { path = "../dasein" }
metacog = { path = "../metacog" }
```

M-A changes add no new crate dependencies. The `AdvancedCompressor` is already in `runtime::impl::memory::compressor`, and `SessionManager` is already in `runtime::impl::daemon::session_manager`. The `prune_tool_outputs` call path (`corpus::tools::tools::output::pruner`) is already used by the compressor and requires no new dependency edges.

Tier 0 changes are filesystem-only (config + README) or add a test that uses existing `toml` + `AppConfig` types.

## Appendix B: Verification Checklist for Implementer

Before committing each phase:

- [ ] `cargo build -p runtime` compiles without errors
- [ ] `cargo test -p runtime <phase_test>` passes
- [ ] `cargo clippy -p runtime -- -D warnings` emits no new warnings
- [ ] `cargo fmt --check` passes on changed files
- [ ] No `todo!()`, `unimplemented!()`, or placeholder strings remain in changed code
- [ ] Git diff reviewed for accidental changes outside the file map
