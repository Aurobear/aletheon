# TUI Scroll Performance and Clarity Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make long Aletheon conversations scroll smoothly and present answers with a concise Codex-style visual hierarchy.

**Architecture:** Cache the fully wrapped static transcript inside `ChatWidget`, invalidating it only when content or width changes, while rendering the small active-tool animation separately. Make the application loop redraw only after state changes or animation ticks, and simplify assistant/Markdown presentation without changing the daemon protocol.

**Tech Stack:** Rust, Ratatui, Crossterm, Tokio, existing TUI test infrastructure.

---

### Task 1: Lock down concise activity behavior

**Files:**
- Modify: `crates/interact/src/tui/chat.rs`
- Modify: `crates/interact/src/tui/response.rs`

- [ ] **Step 1: Correct and extend the existing activity tests**

Assert that a Gmail search renders as `• Searched google_gmail_search`, completed successful output is hidden, failed output exposes at most three lines, and routine on-track reflection produces no entry.

- [ ] **Step 2: Run the focused tests**

Run: `cargo test -p interact test_exec_entry response`

Expected: the stale Gmail expectation fails before correction; all focused tests pass after the implementation is aligned.

- [ ] **Step 3: Commit the completed concise-activity stage**

Commit the semantic activity, reflection suppression, and scoped runner changes together after `git diff --cached --check`.

### Task 2: Remove repetitive answer rails

**Files:**
- Modify: `crates/interact/src/tui/chat.rs`
- Test: `crates/interact/src/tui/chat.rs`

- [ ] **Step 1: Add presentation tests**

Create assistant and user messages and assert assistant lines do not begin with `  │ `, while user input retains a visually distinct but compact prompt treatment.

- [ ] **Step 2: Implement text-first message rendering**

Render assistant Markdown at the available content width without prepending a border span to every line. Keep one blank separator between entries and preserve existing Markdown styles.

- [ ] **Step 3: Verify**

Run: `cargo test -p interact tui::chat`

Expected: all chat rendering and scrolling tests pass.

### Task 3: Cache wrapped transcript layout

**Files:**
- Modify: `crates/interact/src/tui/chat.rs`
- Modify: `crates/interact/src/tui/render/renderable.rs`
- Test: `crates/interact/src/tui/chat.rs`

- [ ] **Step 1: Add cache behavior tests**

Add a test-only rebuild counter. Assert two reads at the same width build once, scrolling does not rebuild, content mutation rebuilds once, and width mutation rebuilds once.

- [ ] **Step 2: Add an interior layout cache**

Store the wrapped lines and their width/revision in `ChatWidget` using interior mutability because Ratatui's render contract receives `&self`. Increment the content revision from `add_text`, `add_exec`, `set_assistant_stream`, `update_exec`, `update_exec_args`, and `toggle_exec`.

- [ ] **Step 3: Render only the visible cached slice**

Expose a cache-backed visible-line method that clamps scroll offsets and returns only the viewport slice. Update `ChatRenderable` and the legacy widget renderer to use it rather than rebuilding and cloning the full transcript.

- [ ] **Step 4: Verify long-history behavior**

Run: `cargo test -p interact tui::chat tui::render`

Expected: cache tests pass and existing snapshots remain valid after intentional presentation updates.

### Task 4: Avoid idle full-screen redraws

**Files:**
- Modify: `crates/interact/src/tui/mod.rs`
- Modify: `crates/interact/src/tui/app/lifecycle.rs`
- Modify: `crates/interact/src/tui/app/key_handler.rs`
- Modify: `crates/interact/src/tui/response.rs`

- [ ] **Step 1: Introduce redraw state**

Add `needs_redraw: bool` to `App`, initialize it to true, and set it after key/mouse/resize handling, socket state changes, submissions, and animation ticks.

- [ ] **Step 2: Gate terminal drawing**

Call `draw_with_recorder` only when `needs_redraw` is set, then clear it. Retain the 50ms animation cadence only while streaming/pending submission; idle polling must not trigger drawing.

- [ ] **Step 3: Verify lifecycle behavior**

Run: `cargo test -p interact tui::app tui::response`

Expected: input, streaming, completion, and test-mode lifecycle tests pass without timeout.

### Task 5: Regression validation and deployment

**Files:**
- Modify if assertions require it: `tools/aletheon-monitor/aletheon_monitor/tui.py`
- Modify if assertions require it: `tools/aletheon-monitor/tests/test_tui.py`

- [ ] **Step 1: Run deterministic validation**

Run:

```bash
cargo fmt --all
cargo test -p interact
cargo test -p corpus security::runner
cargo check -p aletheon-bin
```

Expected: all commands exit zero.

- [ ] **Step 2: Build and install the exact release binary**

Run `cargo build --release -p aletheon-bin`, stop the Aletheon service, install the resulting binary, compare source and installed SHA-256 hashes, and restart the service.

- [ ] **Step 3: Exercise the real TUI three times**

Launch from `/home/aurobear/Bear-ws/aletheon`, submit a repository-analysis task, wait for a stable frame and returned prompt, then scroll a long answer repeatedly. Assert concise semantic activity, substantive final output, correct working directory, and absence of `Ctrl+B to expand`, `lines /`, routine `Reflection:`, false Git errors, and global `safe.directory` advice.

- [ ] **Step 4: Inspect runtime evidence**

Record source commit, installed binary hash, systemd start timestamp, session JSONL, final rendered frame, and journal errors since deployment.

- [ ] **Step 5: Commit validation or monitor changes separately**

If monitor assertions change, commit them as a dedicated test stage after inspecting the staged diff.
