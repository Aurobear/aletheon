# Phase 3 Implementation Plan: Context Management + Tool Enhancement

> **For agentic workers:** task-by-task with `- [ ]` steps. Structured for **multiple developer agents in parallel** — see Dependency Graph and Parallel Batches.

**Goal:** Long conversations don't blow the context window, large tool outputs don't flood it, and the agent gains the missing everyday tools (`web_fetch`, `web_search`, `glob`, `grep`, `task_*`).

**Architecture:** Phase 1's `ReActLoop::run` accumulates `self.messages` with **no compaction and no output budgeting** — the existing `AdvancedCompressor`, `ContextBudget`, and the 4-layer tool-output system are wired only into the orphaned `Engine`. Phase 3 connects that machinery into `ReActLoop`, and adds the missing tools. We **reuse** existing components (compressor, output pruner, sandbox executor) rather than rewrite them.

**Tech Stack:** Rust, tokio, reqwest (already a workspace dep), serde_json.

**Design spec:** [2026-06-19-cli-agent-design.md](./2026-06-19-cli-agent-design.md) §6–§7.

**Verified current state (read-only survey, 2026-06-19):**
- **GAP CONFIRMED:** `ReActLoop::run` (`crates/aletheon-runtime/src/core/react_loop.rs:76-146`) never calls compaction or budget checks. `AdvancedCompressor::maybe_compact` is reachable only from `Engine` (`impl/engine/cognitive_loop.rs:728-738`).
- `AdvancedCompressor` (`impl/memory/compressor/mod.rs`): tail protection, system-message preservation, tool-output pruning (`tools::output::pruner::prune_tool_outputs`), iterative summary. Trigger: `total_tokens >= 2 * tail_token_budget`.
- `ContextBudget` (`impl/memory/budget.rs`): `should_compact()`, `remaining()`, `usage_ratio()`. Exists, **unused**.
- Tool-output system (`body/impl/tools/output/`): capture, truncation (head/tail), persistence (overflow to `/tmp/agentd/overflow/`), `turn_budget.rs` (200k chars/turn), `pruner.rs`. Used by executor, **not linked to ReActLoop message history**.
- Tools present: bash_exec, file_read, file_write, file_search (grep-like content search via ripgrep), apply_patch, process_list, system_status, code_graph, script_tool, tool_search. **Missing:** web_fetch, web_search, glob (file listing), grep (standalone), task_create/update/list/get.
- `Tool` trait (`abi/src/tool.rs:81`): `name/description/input_schema/permission_level/execute/boxed_clone` (+ default `exposure`/`concurrency_class`).
- Sandbox: `SandboxExecutor` + `SandboxPreference` (Auto/Require/Forbid/BestEffort) exist; **no per-tool `SandboxProfile`** (read/write roots, deny paths, network, env).

---

## File Structure & Owner Boundaries

| Agent | Owns (writes) | Responsibility |
|-------|---------------|----------------|
| **A — Compaction** | `crates/aletheon-runtime/src/core/react_loop.rs`, `crates/aletheon-runtime/src/core/config.rs` | Wire `AdvancedCompressor` + `ContextBudget` into `ReActLoop::run`; add reactive compaction on prompt-too-long; add compaction config fields to `RuntimeConfig`. |
| **B — Read tools** | `crates/aletheon-body/src/impl/tools/glob.rs` (NEW), `crates/aletheon-body/src/impl/tools/grep.rs` (NEW), `crates/aletheon-body/src/impl/tools/web_fetch.rs` (NEW), `crates/aletheon-body/src/impl/tools/web_search.rs` (NEW) | Four new L0/L1 tools. |
| **C — Task tools** | `crates/aletheon-body/src/impl/tools/task_tools.rs` (NEW, all four task_* in one file) | `task_create/update/list/get` over an in-memory/SQLite task store. |
| **D — Registry + sandbox profile** | `crates/aletheon-body/src/impl/tools/mod.rs`, `crates/aletheon-body/src/impl/sandbox/profile.rs` (NEW), `crates/aletheon-body/src/impl/sandbox/mod.rs` | Register new tools (B+C) into `ToolRegistry::default()`; add `SandboxProfile`. |

**Shared file:** `tools/mod.rs` is written only by D, who registers the tools B and C create — so B/C create files and D wires them. D depends on B+C landing first.

---

## Dependency Graph

```
Batch 1 (parallel, independent):
  Agent A: A1 (compaction config) → A2 (wire compressor into ReActLoop) → A3 (reactive compact)
  Agent B: B1 (glob) ‖ B2 (grep) ‖ B3 (web_fetch) ‖ B4 (web_search)   [4 independent files]
  Agent C: C1 (task store) → C2 (task_* tools)

Batch 2 (after B + C):
  Agent D: D1 (register new tools in ToolRegistry::default) → D2 (SandboxProfile)

Batch 3:
  E1 (build + tests) → E2 (acceptance: long convo compacts; tools work)
```

Agent A is fully independent (runtime crate). B/C create tool files (body crate) with no
cross-deps. D integrates. The only ordering is D-after-B+C.

---

## Parallel Batches

- **Batch 1:** A, B, C all run concurrently. B's four tools can even be four sub-agents.
- **Batch 2:** D (registration + sandbox profile).
- **Batch 3:** integration agent.

Branch: `auro/feat/20260621-phase3-context-tools`. Commit per task.

---

## Agent A — Compaction into ReActLoop

### Task A1: Compaction config on RuntimeConfig

**Files:**
- Modify: `crates/aletheon-runtime/src/core/config.rs`

- [ ] **Step 1:** Add fields to `RuntimeConfig` (it currently has
  `max_iterations`, `session_id`, `learning_enabled`, `compaction_enabled`):

```rust
    /// Tokens of recent history to always preserve uncompacted.
    pub tail_token_budget: usize,
    /// Target size (chars) of a generated summary.
    pub target_summary_chars: usize,
    /// Context window size for budget tracking.
    pub context_window_tokens: usize,
```
  and set sensible defaults wherever `RuntimeConfig` is constructed (search for
  `RuntimeConfig {` — likely a `Default` impl; add `tail_token_budget: 16_000`,
  `target_summary_chars: 2_000`, `context_window_tokens: 128_000`).

- [ ] **Step 2: Test** — assert defaults:

```rust
#[test]
fn config_has_compaction_defaults() {
    let c = RuntimeConfig::default();
    assert!(c.tail_token_budget > 0 && c.context_window_tokens > c.tail_token_budget);
}
```

Run: `cargo test -p aletheon-runtime config_has_compaction_defaults`
Expected: PASS.

- [ ] **Step 3: Commit** `git commit -am "feat(runtime): compaction config fields on RuntimeConfig"`

---

### Task A2: Wire AdvancedCompressor into ReActLoop::run

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Write the failing test** — a loop fed many large tool results compacts
  below a bound. Use a scripted LLM (reuse the Phase 1 `ScriptedLlm` pattern) that emits N
  tool calls; assert `lp.message_count()` after the run is bounded (compaction ran):

```rust
#[tokio::test]
async fn loop_compacts_when_over_budget() {
    // config with a tiny tail budget so compaction triggers fast
    let cfg = RuntimeConfig { max_iterations: 12, tail_token_budget: 200, target_summary_chars: 100, context_window_tokens: 1000, ..RuntimeConfig::default() };
    let mut lp = ReActLoop::new(cfg);
    // ScriptedLlm: emits a tool call each turn for 10 turns, then ends.
    // execute_tool returns a large (e.g. 5000-char) string each time.
    // After run, assert lp.message_count() is far below 10 full big messages.
    // (exact assertion: messages tokens <= 2 * tail_token_budget after final compaction)
}
```

> Add a `pub fn message_count(&self) -> usize { self.messages.len() }` accessor for the test.

- [ ] **Step 2: Implement** — give `ReActLoop` a compressor and call it each iteration:

  2a. Imports + field:
```rust
  use crate::r#impl::memory::compressor::AdvancedCompressor;
```
```rust
  pub struct ReActLoop {
      config: RuntimeConfig,
      iteration: usize,
      messages: Vec<Message>,
      compressor: AdvancedCompressor,
  }
```
  In `new`: `compressor: AdvancedCompressor::new(config.tail_token_budget, config.target_summary_chars)`.

  2b. In `run`, after pushing tool results back (after the `for (id, name, input)` loop,
  around react_loop.rs:126), call compaction — note `maybe_compact` needs the LLM:
```rust
              // Compact if over budget (reuses the Engine's proven compressor).
              if self.config.compaction_enabled {
                  let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
              }
```
  (Verify `maybe_compact` signature against `compressor/mod.rs`; Phase 1's Engine calls it
  as `self.compressor.maybe_compact(&mut self.messages, &*self.llm).await`.)

- [ ] **Step 3: Test**

Run: `cargo test -p aletheon-runtime react_loop -- --nocapture`
Expected: existing Phase 1 loop test still PASS + `loop_compacts_when_over_budget` PASS.

- [ ] **Step 4: Commit** `git commit -am "feat(runtime): ReActLoop compacts via AdvancedCompressor"`

---

### Task A3: Reactive compaction on prompt-too-long

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1:** Wrap the `llm.complete(...)` call in `run`: if it returns an error whose
  string indicates context overflow (match case-insensitively on `"context"`/`"too long"`/
  `"maximum context"`/`"prompt is too long"`), force one compaction pass and retry once:

```rust
          let response = match llm.complete(&self.messages, tool_defs).await {
              Ok(r) => r,
              Err(e) => {
                  let es = e.to_string().to_lowercase();
                  let overflow = es.contains("context") || es.contains("too long") || es.contains("maximum context");
                  if overflow && self.config.compaction_enabled {
                      warn!("context overflow, forcing compaction + retry");
                      let _ = self.compressor.force_compact(&mut self.messages, llm).await;
                      llm.complete(&self.messages, tool_defs).await?
                  } else {
                      return Err(e);
                  }
              }
          };
```

> If `AdvancedCompressor` has no `force_compact`, add a thin method there that runs the
> summary path unconditionally (or call `maybe_compact` after temporarily halving the
> budget). Check `compressor/mod.rs` and reuse what's there.

- [ ] **Step 2: Test** — scripted LLM that errors "prompt is too long" on first call then
  succeeds; assert the loop recovers and returns text.

Run: `cargo test -p aletheon-runtime react_loop`
Expected: PASS.

- [ ] **Step 3: Commit** `git commit -am "feat(runtime): reactive compaction on context overflow"`

---

## Agent B — Read Tools (4 independent files)

> Each tool implements the `Tool` trait (`abi/src/tool.rs:81`). Pattern to follow:
> read an existing simple tool (`file_read.rs` for L0, `bash_exec.rs` for L1) and match its
> structure exactly — module layout, `async_trait`, `ToolResult`/`ToolResultMeta`.

### Task B1: glob tool

**Files:** Create `crates/aletheon-body/src/impl/tools/glob.rs`

- [ ] **Step 1: Test** — create a temp dir with `a.rs`, `b.txt`, `sub/c.rs`; glob `**/*.rs`
  returns `a.rs` and `sub/c.rs` (relative), not `b.txt`.
- [ ] **Step 2: Implement** `GlobTool` (L0, read-only): input `{ "pattern": string, "root": string? }`;
  use the `walkdir` workspace dep + a glob matcher (`glob` crate — add to
  `aletheon-body/Cargo.toml` if absent) to list matching paths; cap results (e.g. 1000);
  return newline-joined relative paths. `permission_level() = L0`.
- [ ] **Step 3: Test** Run `cargo test -p aletheon-body glob -- --nocapture` → PASS.
- [ ] **Step 4: Commit** `git commit -am "feat(tools): glob file-listing tool (L0)"`

### Task B2: grep tool

**Files:** Create `crates/aletheon-body/src/impl/tools/grep.rs`

- [ ] **Step 1: Test** — temp file with lines; grep a regex returns matching lines with
  line numbers.
- [ ] **Step 2: Implement** `GrepTool` (L0): input `{ "pattern": string, "path": string, "max_results": int? }`.
  Reuse `file_search.rs`'s ripgrep→grep→find fallback approach (read it; you may extract a
  shared helper, but to keep ownership clean, inline the minimal subprocess call here). L0.
- [ ] **Step 3: Test** → PASS.
- [ ] **Step 4: Commit** `git commit -am "feat(tools): standalone grep tool (L0)"`

### Task B3: web_fetch tool

**Files:** Create `crates/aletheon-body/src/impl/tools/web_fetch.rs`

- [ ] **Step 1: Test** — a unit test that constructs the tool and checks `input_schema`
  shape and `permission_level()==L1` (no network in unit tests; mark any live-network test
  `#[ignore]`).
- [ ] **Step 2: Implement** `WebFetchTool` (L1): input `{ "url": string, "method": "GET"|"POST"?, "body": string? }`.
  Use `reqwest` (workspace dep). Cap response size (e.g. 1 MB) and return text; on non-2xx,
  `is_error=true` with status. L1 (network side effect → goes through sandbox/guard).
- [ ] **Step 3: Build** `cargo build -p aletheon-body` → clean.
- [ ] **Step 4: Commit** `git commit -am "feat(tools): web_fetch tool (L1)"`

### Task B4: web_search tool

**Files:** Create `crates/aletheon-body/src/impl/tools/web_search.rs`

- [ ] **Step 1: Test** — schema/permission unit test (live search `#[ignore]`).
- [ ] **Step 2: Implement** `WebSearchTool` (L1): input `{ "query": string, "max_results": int? }`.
  Call a configurable search endpoint (read base URL/key from env, e.g. `SEARCH_API_URL`/
  `SEARCH_API_KEY`); if unconfigured, return a clear `is_error` message ("web search not
  configured") rather than failing the build. L1.
- [ ] **Step 3: Build** → clean.
- [ ] **Step 4: Commit** `git commit -am "feat(tools): web_search tool (L1, env-configured)"`

---

## Agent C — Task Tools

### Task C1: Task store

**Files:** Create `crates/aletheon-body/src/impl/tools/task_tools.rs` (store + tools together)

- [ ] **Step 1: Test** — create→list→get→update round-trip on an in-memory store.
- [ ] **Step 2: Implement** a `TaskStore` (SQLite via `rusqlite`, mirroring
  `learning/outcome.rs`'s pattern, or a simple JSON file under the working dir). Schema:
  `id, subject, description, status (pending|in_progress|completed), created_at, updated_at`.
  Methods: `create`, `get`, `list`, `update_status`.
- [ ] **Step 3: Test** → PASS. **Commit** `git commit -am "feat(tools): task store"`

### Task C2: task_* tools

**Files:** same `task_tools.rs`

- [ ] **Step 1: Test** — `TaskCreateTool` returns an id; `TaskListTool` shows it;
  `TaskUpdateTool` flips status; `TaskGetTool` reflects it.
- [ ] **Step 2: Implement** four `Tool` impls (`task_create`/`task_update`/`task_list`/`task_get`),
  all L0 (they only touch the task store, no system side effects), sharing one
  `Arc<Mutex<TaskStore>>` (or open the store per-call by path). Match the `Tool` trait.
- [ ] **Step 3: Test** → PASS. **Commit** `git commit -am "feat(tools): task_create/update/list/get (L0)"`

---

## Agent D — Registry + Sandbox Profile (Batch 2)

### Task D1: Register new tools

**Files:** Modify `crates/aletheon-body/src/impl/tools/mod.rs`

- [ ] **Step 1:** Add `pub mod glob; pub mod grep; pub mod web_fetch; pub mod web_search; pub mod task_tools;`
- [ ] **Step 2:** In `ToolRegistry::default()` (the builder that registers built-ins — see
  Phase 1 survey, `registry.rs`), register the new tools the same way existing ones are
  registered (`.register(Arc::new(...))`). For task tools sharing a store, construct the
  shared store once and pass it in.
- [ ] **Step 3: Test** — assert `ToolRegistry::default().names()` now contains
  `glob`, `grep`, `web_fetch`, `web_search`, `task_create`, `task_update`, `task_list`,
  `task_get`.

Run: `cargo test -p aletheon-body registry -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit** `git commit -am "feat(tools): register glob/grep/web/task tools"`

### Task D2: SandboxProfile

**Files:** Create `crates/aletheon-body/src/impl/sandbox/profile.rs`; modify `sandbox/mod.rs`

- [ ] **Step 1: Test** — a profile with `network_enabled=false` and a `write_roots` list
  serializes/deserializes and reports `allows_write(path)` correctly.
- [ ] **Step 2: Implement** `SandboxProfile { read_roots, write_roots, deny_paths, network_enabled, env_vars }`
  + helper predicates. This is **additive** — do not change `SandboxExecutor`'s existing
  behavior; the profile is consumed by L1 tools that opt in. (Full enforcement wiring into
  `SandboxExecutor` can be a follow-up; Phase 3 lands the type + predicates + tests.)
- [ ] **Step 3: Test** → PASS. **Commit** `git commit -am "feat(sandbox): SandboxProfile type + predicates"`

---

## Batch 3 — Integration & Acceptance

### Task E1: Build + tests
- [ ] `cargo fmt --all && cargo build --workspace` → clean.
- [ ] `cargo test --workspace` → no failures; count up by the new tests.
- [ ] `cargo clippy --workspace -- -D warnings` → clean (note: new reqwest/glob deps must
  not introduce warnings). Commit fixups.

### Task E2: Defining acceptance test
- [ ] **Step 1: Long conversation doesn't OOM/overflow.** Drive `aletheon-exec` with a
  prompt that forces many large tool outputs (e.g. "read these 30 files and summarize"),
  `--max-turns 30`; confirm it completes without a context-length API error (compaction
  kept it bounded).
- [ ] **Step 2: New tools work.** `aletheon-exec --prompt "Use glob to list all .rs files
  under crates/aletheon-abi, then grep for 'pub struct' in them."` → returns real results,
  files actually listed/searched.
- [ ] **Step 3:** Record outputs in the PR description.

---

## Self-Review (spec coverage)

- Multi-layer compaction wired into ReActLoop → **A2** (tail/system/pruning via AdvancedCompressor) + **A3** (reactive). Config → **A1**.
- New tools glob/grep/web_fetch/web_search → **B1–B4**; task_* → **C1–C2**; registration → **D1**.
- Tool-output budget already exists and is invoked through the compressor's pruner (A2 reuses it). ✓
- Sandbox profile type → **D2** (enforcement wiring noted as follow-up — explicit, not a placeholder).
- Acceptance (long convo bounded; tools work) → **E2**.
- Multi-agent: A=runtime, B=4 tool files, C=task file, D=registry+sandbox; D-after-B+C. ✓

## Notes for implementing agents
- **Reuse, don't rewrite:** A2 must call the existing `AdvancedCompressor`/pruner, not a new compactor.
- **Verify `maybe_compact`/`force_compact` signatures** in `compressor/mod.rs` before coding A2/A3.
- New crate deps (`glob`, possibly a search client) go in `aletheon-body/Cargo.toml`; run `cargo build` to confirm no version conflicts.
- Commit per task; B's four tools are independent and can be four parallel sub-agents.
