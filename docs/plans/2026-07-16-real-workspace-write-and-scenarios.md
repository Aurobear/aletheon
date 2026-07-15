# Real Workspace Writes and Scenario Tests Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow the deployed daemon to deliver files inside the selected project while proving useful, secure behavior through repeatable real-world scenarios.

**Architecture:** Open the workspace root only at the outer systemd namespace, then enforce the narrower canonical client-working-directory boundary in every mutating tool and Bubblewrap invocation. Extend the monitor with authoritative turn completion and a scenario runner that validates the installed daemon, real TUI, host artifacts, safety boundaries, persistence, Gmail, and long-output behavior.

**Tech Stack:** Rust, Tokio, systemd, Bubblewrap, Python/pytest, tmux, JSONL evidence.

---

### Task 1: Reproduce the deployed write failure

**Files:**
- Create: `tools/aletheon-monitor/tests/test_scenarios.py`
- Create: `tools/aletheon-monitor/src/scenarios.py`

- [ ] Add a scenario-result model containing `status`, `assertions`, `evidence`, `duration_ms`, and `failure`.
- [ ] Add a deployment preflight that records canonical source root, HEAD, installed binary SHA-256, unit properties, and daemon timestamp.
- [ ] Add an artifact assertion that checks a uniquely named host file, exact content, and expected project containment.
- [ ] Run the artifact scenario against the current deployment and preserve the expected `Read-only file system` baseline.

### Task 2: Make file mutation project-confined

**Files:**
- Modify: `crates/corpus/src/tools/tools/file_write.rs`
- Modify: `crates/corpus/src/tools/tools/apply_patch.rs`
- Test: inline unit tests in both files

- [ ] Write tests proving relative and absolute paths inside a canonical temporary working directory are accepted.
- [ ] Write tests rejecting sibling directories, `..` traversal, symlink escapes, `.git`, `.env`, `.aletheon`, private-key suffixes, and OAuth client-secret names.
- [ ] Extract a shared canonical mutation-path validator that canonicalizes the nearest existing parent before file creation and rejects protected path components.
- [ ] Use the validator before creating directories, writing files, or applying patches; return a structured tool error on rejection.
- [ ] Run `cargo test -p corpus file_write apply_patch` and require all boundary tests to pass.

### Task 3: Align Bubblewrap and systemd permissions

**Files:**
- Modify: `config/aletheon.service`
- Modify: `crates/corpus/src/security/sandbox/bubblewrap.rs`
- Modify: `scripts/verify-systemd.sh`
- Test: `crates/corpus/src/security/sandbox/bubblewrap.rs`

- [ ] Change the unit from a read-only Bear workspace to `ReadWritePaths=/home/aurobear/Bear-ws` while keeping `ProtectSystem=strict`, `ProtectHome=read-only`, and `/etc/aletheon` read-only.
- [ ] Re-bind existing `.git`, `.env`, `.aletheon`, and credential paths read-only after Bubblewrap binds the validated working directory writable.
- [ ] Add argument-order tests proving root read-only precedes cwd writable and protected paths follow it.
- [ ] Extend systemd verification to fail when strict protection or the bounded workspace source is absent.
- [ ] Run `cargo test -p corpus security::sandbox` plus the systemd verification tests.

### Task 4: Add authoritative real-TUI completion

**Files:**
- Modify: `crates/interact/src/tui/test_infra.rs`
- Modify: `crates/interact/src/tui/response.rs`
- Modify: `tools/aletheon-monitor/src/tools/tui.py`
- Modify: `tools/aletheon-monitor/src/tools/diagnose.py`
- Test: `tools/aletheon-monitor/tests/test_tui_wrappers.py`
- Test: `tools/aletheon-monitor/tests/test_diagnose.py`

- [ ] Record a machine-readable `turn_done` frame/event marker with the active TUI session and turn number in test instrumentation without displaying it to users.
- [ ] Make monitor completion require a post-submit marker advance and a stable frame; remove prompt visibility and fixed quiet periods as completion substitutes.
- [ ] Return session ID, turn number, completion source, and timeout diagnostics.
- [ ] Add tests proving a visible prompt or quiet spinner interval cannot produce a false pass.
- [ ] Run the monitor pytest suite.

### Task 5: Implement realistic scenario coverage

**Files:**
- Modify: `tools/aletheon-monitor/src/scenarios.py`
- Modify: `tools/aletheon-monitor/src/server.py`
- Modify: `tools/aletheon-monitor/README.md`
- Test: `tools/aletheon-monitor/tests/test_scenarios.py`

- [ ] Implement scenarios for repository analysis, artifact delivery, workspace denial, Git awareness, Gmail read-only search, restart recovery, ten-tool completion, and TUI stress.
- [ ] Give every scenario explicit required and forbidden observations; missing OAuth becomes `BLOCKED`, not pass or skip.
- [ ] Register `aletheon_scenarios` with `list` and `run` actions plus scenario selection.
- [ ] Preserve frames, session JSONL, tool results, journal slice, artifact hashes, binary hash, source commit, and unit properties in the result directory.
- [ ] Run deterministic unit tests with fake frames/events and host temporary directories.

### Task 6: Deploy and prove the production path

**Files:**
- Update only if validation exposes defects in the files above.

- [ ] Run `cargo fmt --all`, workspace Clippy with warnings denied, relevant Rust tests, monitor pytest, and release build.
- [ ] Install the updated unit verification helper, systemd unit, and exact release binary; daemon-reload and restart Aletheon.
- [ ] Compare release and installed hashes and verify active unit properties.
- [ ] Run repository analysis, artifact delivery, workspace denial, Git awareness, long-turn completion, and TUI stress three consecutive times through the real TUI.
- [ ] Run Gmail and restart-recovery scenarios once with the production account/state.
- [ ] Confirm the requested Markdown artifact exists on the host, `.git` and sibling writes were denied, no journal warnings occurred, and all completed turns have substantive final answers.
- [ ] Commit implementation, tests, and deployment-polish stages separately after inspecting each staged diff.
