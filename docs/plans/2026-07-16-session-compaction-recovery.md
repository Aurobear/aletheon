# Session Compaction and Recovery Repair Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent UTF-8 compaction panics, bound live model context without rewriting durable history, and guarantee that failed turns release the TUI before deploying the exact tested HEAD.

**Architecture:** Fabric owns UTF-8-safe byte bounding; Cognit keeps the existing raw-event-versus-bounded-active-context split through one shared shaper; Mnemosyne owns summary-plus-tail compaction; Executive propagates compaction failures and converts task failure to the existing `Error` then `TurnDone` terminal sequence; Corpus explains mutation-policy denials accurately; Interact defensively clears active-turn state on errors. Durable journals remain unchanged and deployment is hash-reconciled with the source commit.

**Tech Stack:** Rust 2021, Tokio, serde_json, Cargo tests/Clippy, systemd, tmux-based real TUI monitor.

---

### Task 1: Make Fabric pruning UTF-8 safe

**Files:**
- Modify: `crates/fabric/src/include/compaction.rs:39-128`
- Test: `crates/fabric/src/include/compaction.rs:130-173`

- [ ] **Step 1: Write failing multibyte tests**

Add tests that pass a value longer than 200 bytes containing Chinese and emoji through `truncate_tool_call_args`, then assert the result remains valid, contains the truncation marker, and begins with the longest valid prefix not exceeding 200 bytes. Add an under-budget test that asserts exact equality.

- [ ] **Step 2: Run the focused tests and observe the panic**

Run: `cargo test -p fabric include::compaction::tests -- --nocapture`

Expected: the multibyte case fails at `String::truncate` with `is_char_boundary`.

- [ ] **Step 3: Add and use a byte-boundary helper**

Implement this helper and use it instead of direct byte truncation:

```rust
pub fn truncate_utf8_bytes(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}
```

Capture the original byte length before truncating and make the marker say `bytes`, not `chars`.

- [ ] **Step 4: Run Fabric tests**

Run: `cargo test -p fabric include::compaction::tests`

Expected: all focused tests pass.

- [ ] **Step 5: Commit the UTF-8 repair**

Stage only `crates/fabric/src/include/compaction.rs`, inspect `git diff --cached`, and commit with a conventional subject plus problem/solution body.

### Task 2: Bound current-turn tool results before another model call

**Files:**
- Modify: `crates/cognit/src/harness/linear/tool_exec.rs`
- Modify: `crates/cognit/src/harness/linear/step.rs`
- Create: `crates/cognit/src/harness/linear/tool_output.rs`
- Test: `crates/cognit/tests/cognitive_session.rs`

- [ ] **Step 1: Write failing visibility-versus-durability tests**

Test a pure `bounded_tool_result` helper with ASCII, Chinese, emoji, and under-budget inputs. In the streaming harness, assert the emitted `ToolResult` event retains the full content while the next LLM call receives the bounded head/tail copy.

- [ ] **Step 2: Run focused tests**

Run: `cargo test -p cognit harness::linear::tool_output && cargo test -p cognit --test cognitive_session`

Expected: the helper test fails because the duplicated inline implementations have not been extracted.

- [ ] **Step 3: Implement transient model-visible shaping**

Move the existing UTF-8-safe head/tail logic from both harnesses into `bounded_tool_result(content: &str, max_bytes: usize) -> String`. Report byte counts accurately, preserve the full event emitted before shaping, and place only the bounded copy into the active message buffer. Do not mutate canonical items or journal events.

- [ ] **Step 4: Verify focused tests**

Run the same two commands; expected: pass and the durable source vector remains full-size.

- [ ] **Step 5: Commit context shaping**

Stage only the Cognit files, inspect the staged diff, and commit with a full message body.

### Task 3: Preserve the latest user request and tool boundary across repeated compaction

**Files:**
- Modify: `crates/mnemosyne/src/impl/compressor/tail.rs:20-116`
- Modify: `crates/mnemosyne/src/impl/compressor/mod.rs:64-118`
- Test: `crates/mnemosyne/src/impl/compressor/tail.rs:118-149`
- Test: `crates/mnemosyne/src/impl/compressor/mod.rs:168-263`

- [ ] **Step 1: Add repeated-compaction tests**

Create a history containing an old multibyte tool call/result chain, a newest user message `还是A吧`, and enough content to force compaction twice. Assert after each pass that the latest user message is verbatim, no tail begins with an orphan result, and the second output is deterministic and panic-free.

- [ ] **Step 2: Run Mnemosyne focused tests**

Run: `cargo test -p mnemosyne compressor -- --nocapture`

Expected: at least the latest-user preservation case fails under the current one-message pullback rule.

- [ ] **Step 3: Fix tail selection**

Change latest-user protection to find the last text user request in the whole buffer and lower the cut to that index before the final tool-boundary alignment. Keep the generated summary as the first system message and copy the protected tail verbatim.

- [ ] **Step 4: Re-run focused tests**

Run: `cargo test -p mnemosyne compressor`

Expected: all compressor/tail tests pass twice.

- [ ] **Step 5: Commit compaction semantics**

Stage only Mnemosyne files, inspect, and commit with a full message body.

### Task 4: Propagate compaction failures instead of converting them to `false`

**Files:**
- Modify: `crates/executive/src/impl/daemon/session_manager.rs:189-224`
- Modify: `crates/executive/src/service/turn_runtime_ports.rs:425-466`
- Modify: `crates/executive/src/service/legacy_session_service.rs:300-340`
- Test: `crates/executive/src/impl/daemon/session_manager.rs:471-660`

- [ ] **Step 1: Add a failing-compactor/provider regression test**

Use an LLM stub returning `Err(anyhow!("summary failed"))` and assert `compact_if_needed` returns that error rather than `false`.

- [ ] **Step 2: Run the focused test**

Run: `cargo test -p executive session_manager::compaction_tests`

Expected: fail because `unwrap_or(false)` hides the error.

- [ ] **Step 3: Return `Result<bool>` end to end**

Change `compact_if_needed`, `force_compact`, and `run_compaction` to `Result<bool>`, replace `.unwrap_or(false)` with `?`, propagate from production turn settlement with `?`, and map the legacy manual-compaction error through its existing operation-error helper.

- [ ] **Step 4: Re-run Executive focused tests**

Run: `cargo test -p executive session_manager::compaction_tests && cargo test -p executive --test session_lifecycle_commands`

Expected: pass.

- [ ] **Step 5: Commit fallibility changes**

Stage only the three Executive files and relevant tests, inspect, and commit.

### Task 5: Guarantee a terminal client sequence for failed turn tasks

**Files:**
- Modify: `crates/executive/src/service/turn_pipeline.rs:349-565`
- Modify: `crates/executive/src/impl/daemon/server.rs:172-230`
- Modify: `crates/interact/src/tui/response.rs:144-158`
- Test: `crates/executive/src/service/turn_pipeline.rs`
- Test: `crates/executive/src/impl/daemon/server.rs`
- Test: `crates/interact/src/tui/response.rs`

- [ ] **Step 1: Add failure-completion tests**

Test that a failed/panicked react task emits one `ClientEvent::Error` followed by one `ClientEvent::TurnDone`; test an outer handler JoinError returns a JSON-RPC internal-error response retaining the request ID; test `ClientEvent::Error` clears `streaming`, `waiting`, `app_state.streaming`, and `turn_active`.

- [ ] **Step 2: Run focused tests**

Run: `cargo test -p executive service::turn_pipeline && cargo test -p executive daemon::server && cargo test -p interact tui::response`

Expected: failure because errors are not terminal and the outer JoinHandle is discarded.

- [ ] **Step 3: Implement one terminal emitter and outer panic containment**

Track whether Error and TurnDone were observed during stream drain. When the react result is `Err`, emit missing events in `Error` then `TurnDone` order without duplicates. In the connection loop, retain a handler JoinSet/channel settlement result and serialize a JSON-RPC `-32603` response for JoinError instead of silently losing the request. Set `turn_active = false` in the TUI Error arm.

- [ ] **Step 4: Re-run focused tests**

Run the same commands; expected: pass with exactly one terminal completion.

- [ ] **Step 5: Commit terminal recovery**

Stage only pipeline/server/TUI files and tests, inspect, and commit with a full body.

### Task 6: Make mutation-boundary diagnostics policy-accurate

**Files:**
- Modify: `crates/corpus/src/tools/tools/mutation_path.rs:5-43`
- Modify: `crates/executive/src/service/context_assembler.rs:57-63`
- Modify: `crates/executive/src/impl/runtime/native_cognit.rs:590-599`
- Modify: `crates/corpus/src/tools/mcp/transport.rs:97-123`
- Test: `crates/corpus/src/tools/tools/mutation_path.rs`
- Test: `crates/executive/tests/context_assembler.rs`

- [ ] **Step 1: Add diagnostic contract tests**

For an absolute path outside the working directory, assert the error contains `configured sandbox/working-directory policy`, says host mount state was not checked, recommends relaunching from the intended directory or using an in-workspace path, and contains no actionable mount command. Assert the system prefix carries the same distinction. Add Chinese/emoji tests for the adjacent runtime-error and MCP-name byte bounds.

- [ ] **Step 2: Run the focused test**

Run: `cargo test -p corpus mutation_path && cargo test -p corpus mcp::transport && cargo test -p executive --test context_assembler`

Expected: fail because the current message only says `outside working directory`.

- [ ] **Step 3: Replace the ambiguous diagnostic**

Return a deterministic message identifying policy denial, showing requested and approved roots, stating host mount state was not checked, and listing the two safe remedies. Add a compact system-prefix instruction that sandbox read-only diagnostics outside the approved root do not establish host mount state and host mounts must not be changed. Reuse Fabric's UTF-8 byte helper for native-Cognit runtime errors and MCP tool-name normalization.

- [ ] **Step 4: Re-run Corpus tests**

Run: `cargo test -p corpus mutation_path`

Expected: pass.

- [ ] **Step 5: Commit diagnostics**

Stage only the listed Corpus/Executive diagnostic and truncation files, inspect, and commit with a full body.

### Task 7: Validate the integrated repair

**Files:**
- Verify only; no unrelated edits

- [ ] **Step 1: Format and deletion-audit**

Run: `cargo fmt --all -- --check` and `git diff --diff-filter=D --name-only`.

Expected: formatting passes; deletion output contains only the pre-existing user-owned documentation deletions and no repair-owned file.

- [ ] **Step 2: Run strict crate checks**

Run: `cargo clippy -p fabric -p mnemosyne -p cognit -p corpus -p executive -p interact --all-targets --all-features -- -D warnings`.

Expected: no warnings.

- [ ] **Step 3: Run deterministic tests**

Run: `cargo test -p fabric -p mnemosyne -p cognit -p corpus -p executive -p interact --all-targets --no-fail-fast`.

Expected: all tests pass. If resource pressure occurs, rerun serially with `CARGO_BUILD_JOBS=1` and report both outcomes.

- [ ] **Step 4: Run repository architecture gate**

Run: `scripts/architecture-check.sh`.

Expected: pass with no new findings.

### Task 8: Deploy exact tested HEAD and verify the real TUI three times

**Files:**
- Build: `target/release/aletheon`
- Preserve: installed `/usr/bin/aletheon` backup with timestamp/hash
- Verify: `/etc/aletheon/config.toml` read-only

- [ ] **Step 1: Capture provenance and preserve rollback binary**

Record `git rev-parse HEAD`, `git status --short`, `sha256sum /usr/bin/aletheon`, unit properties, and journal cursor. Copy the installed binary to a timestamped rollback path before replacement; do not edit provider configuration or session journals.

- [ ] **Step 2: Build release from the recorded HEAD**

Run: `cargo build --release -p aletheon-bin` and `sha256sum target/release/aletheon`.

Expected: successful build and a recorded source-binary hash pair.

- [ ] **Step 3: Install and restart**

Install `target/release/aletheon` as `/usr/bin/aletheon`, restart `aletheon.service`, and verify `systemctl is-active aletheon` plus the running executable hash. If startup/smoke fails, restore the preserved binary and restart.

- [ ] **Step 4: Run the real TUI workflow three times**

Launch the actual TUI from `/home/aurobear/Bear-ws/work/kuavo/study/robot-运控`, repeat the formerly failing multibyte/large-output workflow three times, and assert each turn displays a final answer and returns the prompt. RPC-only success is not acceptance evidence.

- [ ] **Step 5: Retain deployment evidence**

Capture final frames, new session IDs/journal paths, daemon journal since restart, absence of `is_char_boundary`/panic/remount guidance, deployed SHA-256, systemd unit properties, and source commit. Confirm the affected historical session journal was neither deleted nor rewritten.
