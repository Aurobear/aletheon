# Client Working Directory and Tester Reliability Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Propagate a validated client working directory through daemon turns and make Aletheon end-to-end testing evidence-based and usable without monitor MCP registration.

**Architecture:** Local clients add `working_dir` to chat RPC parameters. The daemon validates it against the configured Bear-ws workspace root and places it in `TurnRequest`, which is already consumed by the tool pipeline. The tester selects monitor, shipped Python monitor, or tmux/CLI fallback and evaluates explicit assertions.

**Tech Stack:** Rust, Tokio JSON-RPC, systemd, bubblewrap, Python monitor tools, tmux.

---

### Task 1: Client RPC working directory

**Files:**
- Modify: `crates/interact/src/tui/app/submit.rs`
- Modify: `crates/interact/src/tui/app/lifecycle.rs`
- Modify: `crates/interact/src/tui/cli.rs`

- [ ] Add tests asserting chat payloads contain the canonical launch directory.
- [ ] Centralize chat request construction so TUI and `-m` cannot diverge.
- [ ] Add `/cwd` output using the same canonical value.
- [ ] Run `cargo test -p interact` and expect all tests to pass.

### Task 2: Daemon validation and turn propagation

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs`
- Modify: `crates/executive/src/service/daemon_turn/execute.rs`
- Test: `crates/executive/tests/turn_service_equivalence.rs`

- [ ] Add failing tests for a valid Bear-ws directory, nonexistent directory, and `/`.
- [ ] Canonicalize `params.working_dir`, require it to be below `/home/aurobear/Bear-ws`, and return JSON-RPC `-32602` for invalid local paths.
- [ ] Change the local turn entry point to accept the validated path and assign it to `TurnRequest.working_dir`; keep authenticated channel turns on their trusted service directory.
- [ ] Run `cargo test -p executive --lib` and the affected integration tests.

### Task 3: Deployed workspace and sandbox behavior

**Files:**
- Modify: `config/aletheon.service`
- Modify: `crates/corpus/src/security/sandbox/bubblewrap.rs`

- [ ] Add a sandbox execution test for `printf ok >/dev/null && pwd`.
- [ ] Ensure the sandbox creates a writable `/dev/null` for the unprivileged namespace.
- [ ] Make `/home/aurobear/Bear-ws` visible to the daemon while preserving per-command writable binding and existing capability restrictions.
- [ ] Reproduce with `systemd-run` using the production unit properties and expect exit status zero.

### Task 4: Tester and debug fallback

**Files:**
- Modify: `/home/aurobear/Bear-ws/work/aurb/src/skills/general/aletheon-tester/SKILL.md`
- Modify: `/home/aurobear/Bear-ws/work/aurb/src/mcp/aletheon-monitor/src/tools/diagnose.py` if present
- Modify: `/home/aurobear/Bear-ws/work/aurb/src/mcp/aletheon-monitor/src/tui_session.py` if present
- Test: corresponding monitor tests under `/home/aurobear/Bear-ws/work/aurb/src/mcp/aletheon-monitor/tests/`

- [ ] Document capability detection and the three-tier fallback sequence.
- [ ] Capture cwd, git worktree/commit, binary metadata, unit properties, start time, session JSONL, tool errors, and final frame.
- [ ] Replace fixed sleeps with stable-frame plus returned-prompt completion.
- [ ] Add assertion fields for required tools, forbidden strings, expected cwd, final answer, and forbidden host scans.
- [ ] Run the monitor pytest suite and deploy Aurb.

### Task 5: End-to-end deployment verification

**Files:**
- No source changes expected.

- [ ] Build `cargo build --release -p aletheon-bin` and install `/usr/bin/aletheon` plus the unit file.
- [ ] Restart the daemon and record its start timestamp and binary hash.
- [ ] Verify `aletheon -m` from the repository reports the repository cwd.
- [ ] Verify `/dev/null` redirection through `bash_exec`.
- [ ] Run the real TUI cwd task three times and require substantive final answers with no `/opt` scan, sandbox error, or disappearing output.
- [ ] Inspect the new session JSONL and journal entries before reporting success.
