# Streaming Cognitive Loop Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the duplicated streaming/non-streaming ReAct completion paths with one streaming-first engine and a collecting adapter, without changing established turn behavior before the completion gate is enabled.

**Architecture:** `ReActLoop::run_streaming` becomes the only loop implementation. Non-streaming `run` calls the same engine through an internal collecting `EventSink` and a provider adapter that exposes a complete response as stream chunks. Final-response verification, interjection handling, awareness emission, metrics, and terminal emission converge behind one helper. The first increment is behavior-preserving and adds parity tests; later plans add the typed task contract and completion gate at the new seam.

**Tech Stack:** Rust 1.85, Tokio, futures streams, existing `cognit::harness`, Fabric turn events, repository Cargo wrapper.

**Validation cadence:** Build the complete streaming-convergence stage first, while writing characterization and parity tests alongside the code. Do not repeatedly compile after every 2–5 minute edit. Run one narrow deterministic validation batch after Tasks 1–4 are code-complete, then fix failures and rerun the affected target. Installed-runtime deployment is deferred until the larger practical-work phase (completion gate plus core tools) is complete.

---

## File map

- Modify `crates/cognit/src/harness/linear/mod.rs`: declare shared completion helpers and test-only/public adapter exports.
- Create `crates/cognit/src/harness/linear/completion.rs`: own final-answer verification and completion preparation.
- Modify `crates/cognit/src/harness/linear/tool_exec.rs`: call the shared completion helper from the authoritative streaming loop.
- Modify `crates/cognit/src/harness/linear/step.rs`: replace the duplicated loop with a collecting adapter over the authoritative engine.
- Modify `crates/cognit/src/harness/session.rs`: keep public session methods stable and assert both route into the shared engine.
- Test in the same modules, following the existing inline-test pattern.

### Task 1: Characterize current path divergence

**Files:**
- Modify: `crates/cognit/src/harness/linear/mod.rs`
- Modify: `crates/cognit/src/harness/session.rs`

- [ ] **Step 1: Add a verifier parity test fixture**

Add an inline async test using the existing `MockLlmProvider`/test provider style. Configure `RejectOnce`, execute both `CognitiveSession::run_turn` and `run_streaming_turn`, and assert both produce the revised second response and exactly two inference rounds.

```rust
assert_eq!(plain.output, "revised answer");
assert_eq!(streamed.output, plain.output);
assert_eq!(plain.metrics.completed_normally, streamed.metrics.completed_normally);
assert_eq!(provider.calls(), 4); // two calls per path
```

- [ ] **Step 2: Record the expected pre-change failure**

The streaming assertion is expected to fail because `run_streaming` does not invoke the configured verifier. Defer execution to the stage validation batch; the red characterization remains reviewable in the diff.

- [ ] **Step 3: Add an interjection parity characterization**

Use a drain closure that returns one message once, then empty. Assert the collecting adapter and streaming call both consume the interjection before accepting terminal output.

- [ ] **Step 4: Record the second expected pre-change failure**

The collecting/non-streaming assertion is expected to fail because the current `run` path has no interjection drain. Defer execution to the stage validation batch.

- [ ] **Step 5: Commit the red tests**

Commit with subject `test(cognit): characterize loop path divergence` and a body listing verifier and interjection gaps.

### Task 2: Extract the shared finalization seam

**Files:**
- Create: `crates/cognit/src/harness/linear/completion.rs`
- Modify: `crates/cognit/src/harness/linear/mod.rs`
- Modify: `crates/cognit/src/harness/linear/tool_exec.rs`

- [ ] **Step 1: Define the internal decision**

```rust
pub(super) enum FinalizationDecision {
    Continue { assistant_text: Option<String>, interjections: Vec<String> },
    Accept { final_text: String },
}
```

- [ ] **Step 2: Implement one async helper on `ReActLoop`**

```rust
pub(super) async fn finalize_candidate<D, DFut>(
    &mut self,
    final_text: String,
    drain_interjections: &D,
) -> anyhow::Result<FinalizationDecision>
where
    D: Fn() -> DFut,
    DFut: Future<Output = anyhow::Result<Vec<String>>>,
```

The helper drains interjections first. When present, it returns `Continue`. Otherwise it runs the existing verifier with the existing bounded retry counter; rejection appends the assistant candidate and correction request and returns `Continue`. Acceptance returns `Accept`. It does not emit terminal events.

- [ ] **Step 3: Replace the streaming no-tool branch**

Call `finalize_candidate`. On `Continue`, append only the returned interjections not already appended by verifier handling and continue. On `Accept`, emit awareness, `TurnDone`, metrics, and return exactly as today.

- [ ] **Step 4: Inspect the shared-helper diff**

Confirm verifier and interjection ordering are represented once and terminal emission remains outside the helper. Compilation is deferred until the stage is code-complete.

- [ ] **Step 5: Commit**

Commit with subject `refactor(cognit): centralize turn finalization` and a body describing verifier/interjection ordering and unchanged terminal emission.

### Task 3: Make the streaming loop authoritative

**Files:**
- Modify: `crates/cognit/src/harness/linear/step.rs`
- Modify: `crates/cognit/src/harness/linear/tool_exec.rs`
- Modify: `crates/cognit/src/harness/linear/mod.rs`

- [ ] **Step 1: Add `CollectingEventSink`**

```rust
#[derive(Default)]
struct CollectingEventSink {
    terminal: std::sync::Mutex<Option<Result<String, String>>>,
}

impl EventSink for CollectingEventSink {
    fn emit(&self, event: Event) {
        if let Event::TurnDone { result } = event {
            *self.terminal.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        }
    }
}
```

- [ ] **Step 2: Add a complete-response stream adapter**

Implement an internal `LlmProvider` wrapper whose `complete_stream` calls the wrapped provider's `complete` and converts content blocks into ordered `StreamChunk` values followed by `Usage` and `Done`. Reject malformed/truncated tool calls with the same terminal semantics as the streaming provider path.

- [ ] **Step 3: Replace `ReActLoop::run` body**

Preserve the public signature. Seed the user message exactly once, construct the complete-response streaming adapter and no-op interjection drain, call `run_streaming`, and return its outcome. Remove the duplicated inference/tool/finalization loop from `step.rs`.

- [ ] **Step 4: Inspect loop convergence statically**

Confirm `step.rs` contains only the collecting adapter and no second inference/tool loop. Defer execution of parity and harness tests to Task 4.

- [ ] **Step 5: Commit**

Commit with subject `refactor(cognit): make streaming loop authoritative` and explain that non-streaming callers now collect the same event-driven execution.

### Task 4: Complete both host paths, then validate the whole stage

**Files:**
- Modify: `crates/cognit/src/harness/session.rs`
- Test: `crates/executive/src/adapters/runtime/native_cognit.rs`
- Test: `crates/executive/src/application/daemon_react.rs`

- [ ] **Step 1: Add session-level parity tests**

Assert `run_turn` and `run_streaming_turn` produce identical `TurnStop`, output, completion metric, tool count, and error behavior for no-tool, one-tool, verifier-reject, cancellation, and provider-error cases.

- [ ] **Step 2: Add native runtime regression coverage**

Use the existing native runtime test fixtures and assert a verifier rejection cannot be bypassed by the collecting path.

- [ ] **Step 3: Add daemon streaming regression coverage**

Use the existing daemon React fixtures and assert the same verifier behavior plus canonical `TurnDone` ordering.

- [ ] **Step 4: Run the single narrow stage-validation batch**

```bash
bash scripts/cargo-agent.sh test -p cognit --lib
bash scripts/cargo-agent.sh test -p executive native_cognit --lib
bash scripts/cargo-agent.sh test -p executive daemon_react --lib
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: PASS. This is the first compilation/test execution in the stage. Do not run a workspace-wide build.

- [ ] **Step 5: Inspect diff and commit**

Inspect `git diff --check` and the complete staged diff. Commit with subject `test(executive): prove cognitive loop host parity` and a body listing native and daemon evidence.

### Task 5: Stage acceptance boundary

**Files:**
- Modify: `docs/plans/2026-07-26-streaming-cognitive-loop-implementation.md`

- [ ] **Step 1: Mark completed checkboxes and record commands/results**
- [ ] **Step 2: Confirm no completion-gate policy was prematurely introduced**
- [ ] **Step 3: Confirm `step.rs` contains no second model/tool loop**
- [ ] **Step 4: Confirm both host paths end in the same internal engine**
- [ ] **Step 5: Commit the plan evidence**

This slice changes cognition/session behavior, but installed-runtime acceptance is intentionally batched after the larger practical-work phase. That later acceptance must use `sudo bash scripts/aletheon.sh deploy`, binary digest equality, stable systemd restart counters, and a real `/usr/bin/aletheon` request through the official socket. This convergence-only slice stops at deterministic focused validation and must not be reported as installed-runtime acceptance.
