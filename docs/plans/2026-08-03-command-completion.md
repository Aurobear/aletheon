# Command Completion Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make TUI and shell command completion conventional, make bare `/resume` discoverable, and install completion assets during every supported deployment.

**Architecture:** Reuse the TUI command registry and canonical session picker. Keep installed CLI and repository-operations shell definitions separate, then install both from the existing system/rootless deployment boundaries.

**Tech Stack:** Rust, Crossterm, Ratatui, Bash completion, Zsh completion, shell contract tests.

---

### Task 1: TUI completion and resume discovery

**Files:**
- Modify: `crates/interact/src/tui/app/key_handler.rs:380-402`
- Modify: `crates/interact/src/tui/app/submit.rs:175-215`
- Modify: `crates/interact/src/tui/registry.rs:185-204`
- Test: `crates/interact/src/tui/completion.rs`
- Test: `crates/interact/src/tui/app/submit.rs`

- [ ] Add tests proving `Tab` accepts the selected enabled candidate, disabled
  candidates remain rejected, Shift-Tab moves backward, and bare `/resume`
  sends `ClientRpcRequest::Sessions` with `OpenSessionPicker` pending state.
- [ ] Implement one `accept_selected_completion` helper used by Tab and Enter.
- [ ] Route bare `/resume` through the same picker request as `/sessions` while
  retaining direct `ClientRpcRequest::resume(id)` for non-empty IDs.
- [ ] Run `bash scripts/cargo-agent.sh test -p interact completion -- --nocapture`
  and the focused resume tests; expect PASS.
- [ ] Commit as `fix(tui): make command discovery completable` with full context.

### Task 2: Separate CLI and operations shell definitions

**Files:**
- Modify: `scripts/completions/aletheon.bash`
- Modify: `scripts/completions/aletheon.zsh`
- Create: `scripts/completions/aletheon-ops.bash`
- Create: `scripts/completions/aletheon-ops.zsh`
- Test: `tests/suites/operations/completion_test.sh`

- [ ] Move the existing `aletheon.sh` trees into the operations files.
- [ ] Define `aletheon` CLI completion for public top-level and nested Clap
  commands, permission modes, output formats, and typed path options.
- [ ] Add a clean Bash test that sources both files, populates `COMP_WORDS`,
  invokes the registered functions, and asserts expected candidates.
- [ ] Add static Zsh registration and nested-tree assertions.
- [ ] Run `bash tests/suites/operations/completion_test.sh`; expect PASS.
- [ ] Commit as `feat(cli): complete installed command tree` with full context.

### Task 3: Install completions through deploy

**Files:**
- Create: `scripts/libexec/aletheon/install-completions.sh`
- Modify: `scripts/lib/aletheon/install.sh:62-122`
- Modify: `scripts/libexec/aletheon/install-systemd.sh:31-76`
- Modify: `setup.sh:484-506`
- Test: `tests/suites/deployment/completion_install_test.sh`

- [ ] Implement an installer accepting `--system <root>` or `--user <data-home>`
  and installing four mode-0644 assets under standard Bash/Zsh paths.
- [ ] Call it inside system deploy's existing root boundary and rootless deploy's
  user boundary; make setup delegate to the same helper.
- [ ] Test both isolated roots, file modes, registration names, and idempotent
  reinstall.
- [ ] Run the shell tests and `bash scripts/cargo-agent.sh fmt --all -- --check`.
- [ ] Commit as `feat(deploy): install command completions` with full context.
- [ ] Run `sudo bash scripts/aletheon.sh deploy`, verify installed assets,
  matching binary digests, stable restart counters, and a real TUI Tab/resume
  acceptance run.
