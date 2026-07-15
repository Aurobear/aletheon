# Codex-Style Tool Activity Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render concise semantic tool activity and make Git inspection reliable in the sandbox.

**Architecture:** The existing `ExecEntry` remains the source of expandable detail but its collapsed representation becomes a semantic action. The guarded runner injects a working-directory-scoped Git safety configuration. The response handler suppresses no-op reflection events.

**Tech Stack:** Rust, Ratatui, Git environment configuration, bubblewrap.

---

### Task 1: Semantic tool activity

**Files:** `crates/interact/src/tui/chat.rs`

- [ ] Add tests for read, search, shell, and failed action rendering.
- [ ] Replace function-call JSON headers with concise action summaries.
- [ ] Hide successful collapsed stdout and output-size hints.
- [ ] Preserve expanded content and visible failure excerpts.
- [ ] Run `cargo test -p interact tui::chat`.

### Task 2: Reflection noise

**Files:** `crates/interact/src/tui/response.rs`

- [ ] Add a test that on-track continuation reflection is ignored.
- [ ] Keep strategy adjustment, stop, and deviation reflection visible.
- [ ] Run `cargo test -p interact`.

### Task 3: Git sandbox ownership

**Files:** `crates/corpus/src/security/runner.rs`

- [ ] Add a test for working-directory-scoped Git configuration variables.
- [ ] Inject `safe.directory` for only the validated tool working directory.
- [ ] Run `cargo test -p corpus security::runner`.

### Task 4: Real TUI verification

- [ ] Deploy the binary and restart the service.
- [ ] Run project analysis from `/home/aurobear/Bear-ws/aletheon`.
- [ ] Assert Git succeeds, reflection noise is absent, activity is concise, and the final answer remains visible.
