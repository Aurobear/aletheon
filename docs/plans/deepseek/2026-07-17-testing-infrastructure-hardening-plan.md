# Aletheon Testing Infrastructure Hardening Plan

> **Status:** Proposed
>
> **Target branch:** `dev`
>
> **Aletheon baseline:** `e807e41`
>
> **Reference baseline:** local Codex source `1bbdb327`; `TestCodexBuilder`, `ResponseMock`, `insta`, `divan`, `cargo-fuzz` patterns
>
> **Scope:** all workspace crates; test infrastructure, mock providers, snapshot testing, fuzzing, chaos testing, performance benchmarks
>
> **Execution rule:** each phase produces independently mergeable infrastructure. New tests must not break when production code changes shape unless the behavior contract changes. Infrastructure merges before large-scale test writing begins.

## 1. Executive conclusion

Aletheon has a strong test culture with 298 unit tests in `crates/executive/src/` (inline `#[test]` and `#[tokio::test]`), 144 integration tests in `crates/executive/tests/`, and 1,863 `#[test]` annotations across all workspace crates. Every test passes; none are ignored.

The gap is not test quantity but test determinism and integration coverage. Critical production orchestration paths lack direct integration tests:

- `crates/executive/src/service/daemon_turn/execute.rs` (162 lines) -- zero dedicated tests; the daemon turn entry point has no mock-LLM integration coverage.
- `crates/executive/src/service/daemon_react.rs` (129 lines) -- zero dedicated tests; the streaming daemon turn path is untested.
- `crates/executive/src/impl/session/event_sourced_store.rs` (260 lines) -- zero dedicated unit tests; the event-sourced write path is only exercised indirectly through `TurnCoordinator` tests.
- `crates/executive/src/impl/session/canonical_store.rs` (267 lines) -- tested indirectly as infrastructure in 7 test files, but lacks dedicated store-level unit tests for edge cases (corruption, concurrent append races, schema drift).
- `crates/executive/src/service/turn_coordinator.rs` (368 lines) -- has 3 integration tests in `crates/executive/tests/turn_coordinator_lifecycle.rs`, but these cover only the happy path, failure recording, and daemon-then-exec restart. Missing: cancel mid-turn, concurrent turn isolation, event spine projection consistency.

No test anywhere in the workspace uses `#[ignore]`, indicating a missing "known-gap tracking" culture -- there is no mechanism to commit a failing or flaky test with documentation of what must be fixed.

The workspace has zero infrastructure for:
- Deterministic model mocking (no mock LLM provider)
- Snapshot testing (no `insta` or equivalent)
- Fuzzing (no `cargo-fuzz` targets)
- Performance benchmarks (no `criterion` or `divan` benches)
- Chaos/recovery testing (no kill/restart/corruption scenarios)

The target is:

```text
one TestAletheonBuilder that constructs a fully wired, deterministic test instance
one MockLlmProvider with pre-configured turn sequences and assertion hooks
integration tests covering TurnCoordinator, EventSourcedSessionStore, daemon turn, and daemon react
insta snapshot tests for TUI output, protocol messages, context projections, and config schema
cargo-fuzz targets for EnvelopeV2 parsing, JSON-RPC handling, TOML config, and tool input validation
chaos tests for daemon kill/restart, SQLite corruption, and disk-full scenarios
criterion benchmarks for session lifecycle, turn execution, and concurrency profiles
```

## 2. Method and confidence

This plan is based on static inspection of:

- `crates/executive/src/service/turn_coordinator.rs:1-368` -- TurnCoordinator struct and its `submit_with` orchestration
- `crates/executive/src/service/daemon_turn/execute.rs:1-162` -- DaemonTurnOrchestrator::execute_turn
- `crates/executive/src/service/daemon_react.rs:1-129` -- submit_streaming_daemon_turn and DaemonTurnServices
- `crates/executive/src/service/inference_port.rs:1-61` -- InferencePort trait and CoreInferenceRequest
- `crates/executive/src/service/harness_factory.rs:1-105` -- CognitiveSessionFactory trait
- `crates/executive/src/impl/session/canonical_store.rs:1-267` -- CanonicalSessionStore SQLite implementation
- `crates/executive/src/impl/session/event_sourced_store.rs:1-260` -- EventSourcedSessionStore adapter
- `crates/kernel/src/chronos/system_clock.rs:1-74` -- SystemClock and TestClock implementations
- `crates/fabric/src/types/llm_types.rs:1-100` -- LlmProvider trait, LlmResponse, StreamChunk, ToolDefinition
- `crates/fabric/src/types/sandbox.rs:77` -- SandboxBackend trait
- `crates/executive/Cargo.toml:1-53` -- current dependencies and dev-dependencies
- `crates/executive/tests/turn_coordinator_lifecycle.rs:1-266` -- existing coordinator tests
- `crates/executive/tests/session_append_store.rs:1-183` -- existing canonical store tests
- every test file in `crates/executive/tests/` (103 files)
- local Codex `TestCodexBuilder` patterns, `mount_sse_once`/`mount_sse_sequence`, `ResponseMock`, `insta`, and `divan` usage

Test counts were verified via `grep -rn '#\[test\]' crates/executive/src/ --include="*.rs" | wc -l` (298) and `grep -rn '#\[test\]' crates/executive/tests/ --include="*.rs" | wc -l` (144). The zero-`#[ignore]` claim was verified via `grep -rn '#\[ignore\]' crates/ --include="*.rs"`.

## 3. Current test infrastructure landscape

### 3.1 Test file inventory

| Location | Count | Type |
|---|---|---|
| `crates/executive/src/` | 298 `#[test]` | Inline unit tests |
| `crates/executive/tests/` | 144 `#[test]` | Integration tests (103 files) |
| All workspace crates combined | 1,863 `#[test]` | All tests |

### 3.2 Current test patterns

Existing tests in `crates/executive/tests/` use these patterns, which the new infrastructure must compose with:

- In-memory SQLite via `CanonicalSessionStore::open(":memory:")` -- used in `turn_coordinator_lifecycle.rs:44`, `session_lifecycle_commands.rs:32`, `principal_turn_isolation.rs:30`, and 6 more files
- File-backed SQLite via `tempfile::tempdir()` -- used in `session_append_store.rs:29` for restart-durability tests
- `Arc<KernelRuntime>` for process/operation lifecycle -- used in `turn_coordinator_lifecycle.rs:42`
- `Arc<dyn SessionAppendStore>` as the store boundary -- used throughout
- `SqliteEventSpine::open(":memory:")` for event spine -- used in `turn_coordinator_lifecycle.rs:45`
- Custom `CognitiveSessionFactory` implementations for seed message capture -- `SeedCapturingFactory` in `turn_coordinator_lifecycle.rs:169-180`
- `EmptyServices` struct implementing `TurnServices` -- used in `turn_coordinator_lifecycle.rs:208-230`
- `turn_request_support` module for constructing `TurnRequest` values

### 3.3 Critical gaps

| Production file | Lines | Direct tests | Gap |
|---|---|---|---|
| `service/daemon_turn/execute.rs` | 162 | 0 | Full daemon turn entry point, JSON-RPC response wrapping, error handling |
| `service/daemon_react.rs` | 129 | 0 | Streaming turn path, DaemonTurnServices construction, CognitiveSessionFactory usage |
| `impl/session/event_sourced_store.rs` | 260 | 0 | Event append, materialization, poison detection, writer serialization |
| `impl/session/canonical_store.rs` | 267 | 0 dedicated | Edge cases: concurrent append, schema validation, gap detection, fork edge cases |
| `service/turn_coordinator.rs` | 368 | 3 | Cancel mid-turn, concurrent turns, event spine projection consistency, active turn index |

### 3.4 Missing infrastructure

| Capability | Codex has | Aletheon has |
|---|---|---|
| Deterministic mock LLM | `mount_sse_once` / `mount_sse_sequence` | None |
| Test builder pattern | `TestCodexBuilder` (45K lines) | None |
| Outbound request assertion | `ResponseMock` | None |
| Snapshot testing | `insta` crate | None |
| Performance benchmarks | `divan` crate | None |
| Fuzzing | `cargo-fuzz` (libfuzzer) | None |
| Chaos/recovery tests | Process kill/restart fixtures | None |
| Known-gap tracking | `#[ignore]` with reason | Zero `#[ignore]` |

## 4. Architecture principles for test infrastructure

### 4.1 Adopt from Codex: deterministic test composition

Codex demonstrates several patterns directly applicable to Aletheon:

- **Builder pattern**: `TestCodexBuilder` constructs a fully wired test instance with mock HTTP servers, fake auth, and configurable environments. Aletheon needs `TestAletheonBuilder` that constructs the equivalent: mock LLM, mock sandbox, test clock, test stores.
- **SSE event stream mocking**: `mount_sse_once` and `mount_sse_sequence` inject deterministic model responses via SSE events. Aletheon's `MockLlmProvider` should serve pre-configured `LlmResponse` and `LlmStream` sequences.
- **Outbound request capture**: `ResponseMock` captures and asserts against API requests. Aletheon's mock provider should expose the messages and tool definitions sent to the "model" for assertion.
- **Snapshot testing**: `insta` with inline review workflow. Aletheon should snapshot TUI output, JSON-RPC protocol messages, context projections, and config schema.
- **Benchmarks**: `divan` for ergonomic, attribute-based benchmarks. Aletheon should benchmark session lifecycle, turn execution, and concurrency.
- **Clock injection**: `TestClock` already exists in `crates/kernel/src/chronos/system_clock.rs:41` with `advance()` method. The gap is that no integration test constructs a full system with an injected clock.

### 4.2 Binding rules

1. Test infrastructure lives in `crates/executive/tests/support/` as shared modules, not as a separate crate (avoids circular dependencies and crate-boundary friction).
2. Mock implementations implement existing Fabric traits (`LlmProvider`, `SandboxBackend`, `Clock`); no new traits needed for testing.
3. Builder pattern: one `TestAletheonBuilder` constructs the full testable stack. Individual tests call `.with_mock_llm(sequence)` or `.with_test_clock(clock)` for specialization.
4. Test infrastructure must not depend on production-only crates (no `reqwest` in test support, no real network).
5. Snapshot tests update automatically with `cargo insta review`; CI fails on unstaged snapshot changes.
6. Fuzz targets live in `crates/executive/fuzz/` (standard `cargo-fuzz` layout).
7. Benchmarks live in `crates/executive/benches/` (standard `criterion` layout).
8. Chaos tests are integration tests gated behind `#[cfg(feature = "chaos")]` since they require process management and are inherently slower.

## 5. TestAletheonBuilder design

### 5.1 Type definition

Located at `crates/executive/tests/support/test_aletheon_builder.rs`:

```rust
use std::sync::Arc;
use aletheon_kernel::KernelRuntime;
use aletheon_kernel::chronos::TestClock;
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use executive::r#impl::events::SqliteEventSpine;
use executive::service::turn_coordinator::TurnCoordinator;
use fabric::{SessionAppendStore, Clock, EventSpine};

/// Pre-configured turn sequence for MockLlmProvider.
pub struct MockTurnSequence {
    /// One or more responses returned in order. When exhausted, the
    /// provider panics (test bug: under-specified sequence).
    pub responses: Vec<MockTurnResponse>,
}

pub enum MockTurnResponse {
    /// A single complete() response.
    Complete {
        content: Vec<fabric::ContentBlock>,
        stop_reason: fabric::StopReason,
        usage: fabric::Usage,
    },
    /// A streamed response: chunks delivered sequentially.
    Stream {
        chunks: Vec<fabric::StreamChunk>,
    },
    /// Simulate a model error (rate limit, overload, auth failure).
    Error {
        message: String,
    },
    /// Simulate a timeout (provider never responds).
    Timeout,
}

/// Builder for a fully wired, deterministic test Aletheon instance.
pub struct TestAletheonBuilder {
    kernel: Option<Arc<KernelRuntime>>,
    clock: Option<Arc<TestClock>>,
    canonical_store: Option<Arc<CanonicalSessionStore>>,
    event_spine: Option<Arc<SqliteEventSpine>>,
    llm_sequences: Vec<MockTurnSequence>,
    tool_results: std::collections::HashMap<String, Vec<MockToolResult>>,
    session_store_path: Option<std::path::PathBuf>,
}

pub struct MockToolResult {
    pub output: String,
    pub is_error: bool,
}

/// A fully constructed test Aletheon, ready for test execution.
pub struct TestAletheon {
    pub kernel: Arc<KernelRuntime>,
    pub clock: Arc<TestClock>,
    pub store: Arc<dyn SessionAppendStore>,
    pub event_spine: Arc<dyn EventSpine>,
    pub coordinator: TurnCoordinator,
    pub llm_provider: Arc<MockLlmProvider>,
    pub sandbox: Arc<MockSandbox>,
    /// Handle for inspecting internal state after test execution.
    pub inspector: TestInspector,
}

/// Inspection handles for asserting on internal state.
pub struct TestInspector {
    pub llm_provider: Arc<MockLlmProvider>,
    pub clock: Arc<TestClock>,
    pub store: Arc<dyn SessionAppendStore>,
    pub event_spine: Arc<dyn EventSpine>,
}
```

### 5.2 Builder API

```rust
impl TestAletheonBuilder {
    /// Create a new builder with all-memory defaults.
    pub fn new() -> Self { ... }

    /// Use a specific TestClock instead of the default (wall=0, mono=0).
    pub fn with_clock(mut self, clock: Arc<TestClock>) -> Self { ... }

    /// Provide pre-configured LLM turn sequences. Each call to
    /// complete() or complete_stream() consumes the next response
    /// in the current sequence. When a sequence is exhausted,
    /// the provider moves to the next sequence.
    pub fn with_llm_sequences(mut self, sequences: Vec<MockTurnSequence>) -> Self { ... }

    /// Register mock tool results keyed by tool name. The mock
    /// sandbox returns these results in FIFO order per tool.
    pub fn with_tool_results(
        mut self,
        results: std::collections::HashMap<String, Vec<MockToolResult>>,
    ) -> Self { ... }

    /// Use a file-backed session store instead of in-memory.
    /// Required for restart/durability tests.
    pub fn with_persistent_store(mut self, path: std::path::PathBuf) -> Self { ... }

    /// Build the fully wired test instance.
    pub async fn build(self) -> TestAletheon { ... }

    /// Build with a single "happy path" LLM response that returns
    /// the given text. Convenience for simple tests.
    pub async fn build_simple(self, assistant_response: &str) -> TestAletheon { ... }
}
```

### 5.3 Builder internals

The builder constructs the following object graph:

```text
TestClock
  -> KernelRuntime::with_clock(test_clock)
  -> TurnCoordinator::new(kernel, canonical_store)
MockLlmProvider<sequences>
  -> Arc<dyn LlmProvider>
MockSandbox<tool_results>
  -> Arc<dyn SandboxBackend>
CanonicalSessionStore (in-memory or file-backed)
  -> Arc<dyn SessionAppendStore>
SqliteEventSpine (in-memory)
  -> Arc<dyn EventSpine>
```

The `kernel` field in `TurnCoordinator` currently constructs its own `SystemClock` inside `KernelRuntime::new()`. `KernelRuntime` must gain a `with_clock` constructor (or accept `Arc<dyn Clock>` as a parameter) for deterministic time injection. This is tracked in the implementation phases below.

### 5.4 Usage example

```rust
#[tokio::test]
async fn daemon_turn_with_tool_call_produces_correct_canonical_items() {
    let tool_results = std::collections::HashMap::from([
        ("bash".into(), vec![MockToolResult {
            output: "file.txt\n".into(),
            is_error: false,
        }]),
    ]);

    let llm_sequences = vec![MockTurnSequence {
        responses: vec![
            MockTurnResponse::Complete {
                content: vec![fabric::ContentBlock::ToolUse {
                    id: "tool_1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                }],
                stop_reason: fabric::StopReason::ToolUse,
                usage: fabric::Usage::default(),
            },
            MockTurnResponse::Complete {
                content: vec![fabric::ContentBlock::Text {
                    text: "Here are the files: file.txt".into(),
                }],
                stop_reason: fabric::StopReason::EndTurn,
                usage: fabric::Usage::default(),
            },
        ],
    }];

    let test = TestAletheonBuilder::new()
        .with_llm_sequences(llm_sequences)
        .with_tool_results(tool_results)
        .build()
        .await;

    // Exercise: submit a turn through TurnCoordinator
    // Assert: canonical items contain UserMessage, ToolCall, ToolResult, AssistantMessage
    // Assert: operation state is Succeeded

    // Inspect what the mock LLM received
    let messages_sent = test.inspector.llm_provider.last_messages();
    assert!(messages_sent.iter().any(|m| matches!(m.role, fabric::Role::User)));
}
```

## 6. MockLlmProvider design

### 6.1 Type definition

Located at `crates/executive/tests/support/mock_llm_provider.rs`:

```rust
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use async_trait::async_trait;
use fabric::{LlmProvider, LlmResponse, LlmStream, Message, ToolDefinition, StreamChunk, StopReason, Usage};
use futures::stream;

/// Deterministic mock LLM provider for integration tests.
///
/// Pre-configured with sequences of responses. Each call to
/// `complete()` or `complete_stream()` advances an internal cursor
/// through the current sequence. If no responses remain, the
/// provider panics to signal a test that did not specify enough turns.
///
/// Also records all messages and tool definitions sent to it for
/// post-turn assertion.
pub struct MockLlmProvider {
    sequences: Mutex<Vec<MockTurnSequence>>,
    /// Index into `sequences` for the current active sequence.
    sequence_index: AtomicUsize,
    /// Index into the current sequence's responses.
    response_index: AtomicUsize,
    /// Record of the most recent complete() call arguments.
    last_messages: Mutex<Vec<Message>>,
    last_tools: Mutex<Vec<ToolDefinition>>,
    /// Full history of all complete() calls.
    message_history: Mutex<Vec<Vec<Message>>>,
    /// Number of times complete() was called.
    call_count: AtomicUsize,
}
```

### 6.2 LlmProvider trait implementation

```rust
#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        // Record for assertion
        *self.last_messages.lock().unwrap() = messages.to_vec();
        *self.last_tools.lock().unwrap() = tools.to_vec();
        self.message_history.lock().unwrap().push(messages.to_vec());
        self.call_count.fetch_add(1, Ordering::SeqCst);

        let seq_idx = self.sequence_index.load(Ordering::SeqCst);
        let resp_idx = self.response_index.fetch_add(1, Ordering::SeqCst);

        let sequences = self.sequences.lock().unwrap();
        let sequence = sequences.get(seq_idx)
            .unwrap_or_else(|| panic!(
                "MockLlmProvider: no sequence at index {seq_idx}. \
                 Test must configure enough turn sequences."
            ));

        let response = sequence.responses.get(resp_idx)
            .unwrap_or_else(|| panic!(
                "MockLlmProvider: sequence {seq_idx} exhausted at response {resp_idx}. \
                 Expected more responses in this turn sequence."
            ));

        match response {
            MockTurnResponse::Complete { content, stop_reason, usage } => {
                Ok(LlmResponse {
                    content: content.clone(),
                    stop_reason: *stop_reason,
                    usage: usage.clone(),
                    cache_hit_tokens: 0,
                })
            }
            MockTurnResponse::Error { message } => {
                anyhow::bail!("mock provider error: {message}")
            }
            MockTurnResponse::Timeout => {
                // Simulate timeout by never resolving (test should use tokio::time::timeout)
                std::future::pending::<()>().await;
                unreachable!()
            }
            MockTurnResponse::Stream { chunks } => {
                // For complete(), concatenate text chunks and collect tool calls
                self.collect_from_stream(chunks)
            }
        }
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        // Record, then return a stream of pre-configured chunks
        // ... (similar recording + stream construction)
    }

    fn name(&self) -> &str { "mock" }
    fn max_context_length(&self) -> usize { 200_000 }
}
```

### 6.3 Inspection API

```rust
impl MockLlmProvider {
    /// Return the messages sent in the most recent complete() call.
    pub fn last_messages(&self) -> Vec<Message> {
        self.last_messages.lock().unwrap().clone()
    }

    /// Return all message batches sent across all complete() calls.
    pub fn message_history(&self) -> Vec<Vec<Message>> {
        self.message_history.lock().unwrap().clone()
    }

    /// Return the tool definitions from the most recent call.
    pub fn last_tools(&self) -> Vec<ToolDefinition> {
        self.last_tools.lock().unwrap().clone()
    }

    /// Total number of complete() invocations.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    /// Assert that the Nth message batch contains a message matching
    /// the predicate. Returns the matched message.
    pub fn assert_message<F>(&self, call_index: usize, predicate: F) -> &Message
    where F: Fn(&Message) -> bool { ... }
}
```

### 6.4 Pre-configured sequence factories

```rust
impl MockLlmProvider {
    /// Single-turn text-only response.
    pub fn single_text_response(text: &str) -> Self {
        Self::from_sequences(vec![MockTurnSequence {
            responses: vec![MockTurnResponse::Complete {
                content: vec![fabric::ContentBlock::Text { text: text.into() }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            }],
        }])
    }

    /// Multi-turn conversation: alternating tool calls and text.
    pub fn multi_tool_turns(
        turns: Vec<Vec<MockTurnResponse>>,
    ) -> Self { ... }

    /// Provider that always errors.
    pub fn always_error(message: &str) -> Self {
        Self::from_sequences(vec![MockTurnSequence {
            responses: vec![MockTurnResponse::Error {
                message: message.into(),
            }],
        }])
    }
}
```

## 7. MockSandbox design

### 7.1 Type definition

Located at `crates/executive/tests/support/mock_sandbox.rs`:

```rust
use std::sync::Mutex;
use async_trait::async_trait;
use fabric::{SandboxBackend, SandboxConfig, SandboxCommand, SandboxCapabilities,
             SandboxResult, IsolationLevel};

pub struct MockSandbox {
    /// FIFO queue of results per tool name.
    results: Mutex<std::collections::HashMap<String, Vec<MockToolResult>>>,
    /// All execute calls for post-turn assertion.
    execution_log: Mutex<Vec<MockExecutionRecord>>,
}

pub struct MockExecutionRecord {
    pub tool_name: String,
    pub args: Vec<String>,
    pub config: SandboxConfig,
}

impl MockSandbox {
    pub fn new(
        results: std::collections::HashMap<String, Vec<MockToolResult>>,
    ) -> Self { ... }

    /// All execution records for assertion.
    pub fn execution_log(&self) -> Vec<MockExecutionRecord> { ... }
}

#[async_trait]
impl SandboxBackend for MockSandbox {
    fn name(&self) -> &str { "mock" }
    fn isolation_level(&self) -> IsolationLevel { IsolationLevel::None }
    fn is_available(&self) -> bool { true }
    fn capabilities(&self) -> SandboxCapabilities { ... }

    async fn execute(
        &self,
        command: &SandboxCommand,
        _config: &SandboxConfig,
    ) -> anyhow::Result<SandboxResult> {
        let tool_name = command.program.to_string_lossy().to_string();
        self.execution_log.lock().unwrap().push(MockExecutionRecord {
            tool_name: tool_name.clone(),
            args: command.args.clone(),
            config: _config.clone(),
        });
        let mut results = self.results.lock().unwrap();
        let queue = results.get_mut(&tool_name)
            .unwrap_or_else(|| panic!("MockSandbox: no results configured for tool '{tool_name}'"));
        let result = queue.remove(0);
        Ok(SandboxResult {
            stdout: result.output.clone(),
            stderr: String::new(),
            exit_code: if result.is_error { 1 } else { 0 },
            backend_used: "mock".into(),
            isolation_level: IsolationLevel::None,
            elapsed_ms: 0,
        })
    }
}
```

## 8. Integration test suite plan

### 8.1 TurnCoordinator tests

File: `crates/executive/tests/turn_coordinator_integration.rs`

These tests extend the existing 3 tests in `turn_coordinator_lifecycle.rs`. The new tests use `TestAletheonBuilder` and `MockLlmProvider` for determinism.

| Test | Scenario | Assertions |
|---|---|---|
| `create_session_on_first_turn` | Submit turn with new session ID | Session created in store, `SessionCreated` event in spine |
| `append_items_in_sequence_order` | Turn produces ToolCall + ToolResult + AssistantMessage | Items at sequences 2,3,4 (after UserMessage at 1); no gaps |
| `settle_operation_on_success` | Turn completes normally | Operation state=Succeeded; terminal item is AssistantMessage |
| `settle_operation_on_failure` | Runner returns Err | Operation state=Failed; SystemNotice appended; turn result is error |
| `cancel_mid_turn` | Cancel token fired during runner execution | Operation state=Cancelled; CancelReason recorded |
| `cancel_before_start` | Cancel token fired before runner starts | Turn never executes; no items appended beyond UserMessage |
| `timeout_deadline` | Deadline set, clock advances past it during turn | Operation cancelled with DeadlineExceeded reason |
| `concurrent_turns_different_sessions` | Two turns on different sessions submitted concurrently | Both complete; operations independent; active index correct |
| `concurrent_turns_same_principal_rejected` | Two turns on same principal | Second turn is queued or rejected; active index enforces exclusivity |
| `event_spine_sequence_monotonic` | Multiple turns on same session | Spine sequence strictly increasing; TreeSequence monotonic |
| `context_projection_stored_as_item` | Turn returns ContextProjectionReceipt | ItemPayload::ContextProjection at correct sequence position |
| `idempotent_item_append` | Duplicate sequence append | AppendOutcome::AlreadyPresent; no duplicate in load |
| `deadline_from_request` | TurnRequest with deadline set | MonoDeadline calculated from clock.mono_now() + deadline |

### 8.2 EventSourcedSessionStore tests

File: `crates/executive/tests/event_sourced_store_tests.rs`

| Test | Scenario | Assertions |
|---|---|---|
| `append_produces_spine_event` | Append item through event-sourced store | SpineEvent appended; correct SchemaId and EventTreeId |
| `materialize_to_read_model` | Append item, then load from read model | Item present; sequence correct; session created if new |
| `idempotent_replay` | Append same item twice through event-sourced store | Second append returns AlreadyPresent; spine has one event |
| `projection_consistency` | Append items, check projection state | SessionProjection reflects all appended items |
| `poison_detection` | Inject a projection that always fails | Poison reported in ProjectionReport; main append still succeeds |
| `writer_serialization` | Two concurrent appends | Second append waits; both succeed; sequences sequential |
| `fork_through_event_sourcing` | Fork session via event-sourced store | Items copied to child; new ItemIds; ForkEvent in spine |
| `session_creation_event` | Create new session via event-sourced store | SessionCreatedV1 event in spine at sequence 1 |

### 8.3 Daemon turn integration tests

File: `crates/executive/tests/daemon_turn_integration.rs`

These tests exercise `DaemonTurnOrchestrator::execute_turn` with the full `TestAletheon` stack.

| Test | Scenario | Assertions |
|---|---|---|
| `full_daemon_turn_text_response` | Submit turn, mock returns text | JSON-RPC result with `response` field; operation succeeded |
| `full_daemon_turn_with_tool_call` | Mock returns tool call then text | Tool executed via mock sandbox; both items in canonical store |
| `full_daemon_turn_model_error` | Mock returns provider error | JSON-RPC error response; operation failed |
| `full_daemon_turn_kernel_error` | Kernel process registration fails | JSON-RPC error response with -32603 code |
| `authenticated_turn_preserves_principal` | Use execute_authenticated_turn | PrincipalId from context binding, not from input |
| `daemon_turn_operation_kind` | Verify operation kind | Should be OperationKind::Turn (not SubAgent -- tracked as issue) |
| `context_projection_in_response` | Mock returns conscious context | ContextProjection extracted from response JSON; stored as item |

### 8.4 Daemon react tests

File: `crates/executive/tests/daemon_react_tests.rs`

| Test | Scenario | Assertions |
|---|---|---|
| `streaming_turn_builds_services` | Construct DaemonTurnServices with mock LLM | LlmProvider accessible; tool_defs correct |
| `streaming_turn_tool_execution` | Execute tool through DaemonTurnServices::invoke | Tool closure called; output captured; is_error propagated |
| `streaming_turn_seed_messages` | Request seed messages | Returns pre-configured request_messages |
| `streaming_turn_cancellation` | Cancel token fired | Session run_streaming_turn returns error |

### 8.5 CanonicalSessionStore edge case tests

File: `crates/executive/tests/canonical_store_edge_cases.rs`

| Test | Scenario | Assertions |
|---|---|---|
| `schema_version_rejection` | Create session with wrong schema version | Error with "unsupported session schema version" |
| `gap_detection_on_append` | Append at sequence 3 when last is 1 | Error; gap not silently filled |
| `concurrent_append_same_sequence` | Two appends at same sequence | One succeeds (Appended), one errors or gets AlreadyPresent |
| `fork_beyond_parent_items` | Fork through_sequence > parent item count | Error or bounded to max available |
| `load_items_with_from_filter` | Load items starting from specific sequence | Only items at or after that sequence returned |
| `session_not_found` | Load session for non-existent ID | None returned; no panic |
| `corrupt_json_in_database` | Manually insert invalid JSON into item_json column | load_items returns error; does not panic |

## 9. Snapshot testing

### 9.1 Target files

| Target | Location | What is snapshotted |
|---|---|---|
| TUI rendering | `crates/interact/tests/snapshots/` | Full terminal output for key interaction sequences |
| JSON-RPC protocol messages | `crates/executive/tests/snapshots/` | Request/response pairs for all RPC handlers |
| Context projections | `crates/executive/tests/snapshots/` | `ContextProjectionReceipt` values sent to model |
| Config schema generation | `crates/executive/tests/snapshots/` | Generated JSON Schema for ExecutiveConfig |

### 9.2 insta setup

Add to `crates/executive/Cargo.toml` `[dev-dependencies]`:

```toml
insta = { version = "1", features = ["json", "toml"] }
```

Add to `crates/interact/Cargo.toml` `[dev-dependencies]`:

```toml
insta = { version = "1", features = ["json"] }
```

### 9.3 Snapshot test examples

```rust
// crates/executive/tests/snapshot_context_projection.rs
#[test]
fn context_projection_is_deterministic() {
    let receipt = fabric::ContextProjectionReceipt {
        space: fabric::AgoraSpaceId("test-space".into()),
        broadcast_epoch: Some(fabric::BroadcastEpoch(2)),
        workspace_version: Some(3),
        dasein_version: fabric::dasein::SelfVersion(4),
        content_ids: vec![fabric::ContentId(uuid::Uuid::from_u128(5))],
    };
    insta::assert_json_snapshot!(receipt);
}

// crates/executive/tests/snapshot_config_schema.rs
#[test]
fn config_schema_is_stable() {
    let schema = schemars::schema_for!(executive::core::config::ExecutiveConfig);
    insta::assert_json_snapshot!(schema);
}
```

### 9.4 CI integration

```bash
# In CI: verify snapshots are up to date
cargo insta test --workspace --accept-unseen

# Locally: review and accept snapshot changes
cargo insta review
```

Add to CI workflow: `cargo insta test --workspace` fails if any snapshot is missing or out of date.

## 10. Fuzzing strategy

### 10.1 Fuzz targets

Located in `crates/executive/fuzz/fuzz_targets/`:

| Target | Input type | What it tests |
|---|---|---|
| `envelope_v2_parse` | Arbitrary bytes | `EnvelopeV2::from_slice` -- must not panic, must return error for malformed input |
| `envelope_v2_roundtrip` | Structured `EnvelopeV2` | serialize then deserialize must be identity |
| `jsonrpc_message_parse` | Arbitrary bytes | JSON-RPC request/response parsing -- must not panic |
| `jsonrpc_method_dispatch` | Structured method name + params | Method routing must not panic on unknown methods |
| `toml_config_parse` | Arbitrary bytes | TOML config parsing -- must not panic, graceful errors |
| `tool_input_json` | Arbitrary bytes | Tool JSON schema validation -- must not panic on malformed inputs |
| `message_roundtrip` | Structured `Message` | Serialize/deserialize fabric::Message |

### 10.2 Setup

Add to `crates/executive/Cargo.toml`:

```toml
[package]
# ... existing ...

# cargo-fuzz does not use a dev-dependency; it uses workspace
# members or is configured via .cargo/config.toml.

# Fuzz targets are compiled with:
#   cargo fuzz run <target> --fuzz-dir crates/executive/fuzz
```

Create `crates/executive/fuzz/Cargo.toml`:

```toml
[package]
name = "executive-fuzz"
version = "0.0.0"
edition = "2021"
publish = false

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
arbitrary = { version = "1", features = ["derive"] }

[dependencies.executive]
path = ".."

[dependencies.fabric]
path = "../../fabric"
```

### 10.3 Fuzz target example

```rust
// crates/executive/fuzz/fuzz_targets/envelope_v2_parse.rs
#![no_main]

use libfuzzer_sys::fuzz_target;
use fabric::EnvelopeV2;

fuzz_target!(|data: &[u8]| {
    // Must never panic. Invalid input must return Err.
    let _ = EnvelopeV2::from_slice(data);
});
```

### 10.4 CI integration

```bash
# Run each fuzz target for a fixed time (quick-check in CI)
for target in envelope_v2_parse envelope_v2_roundtrip jsonrpc_message_parse \
              jsonrpc_method_dispatch toml_config_parse tool_input_json \
              message_roundtrip; do
    cargo fuzz run "$target" --fuzz-dir crates/executive/fuzz -- -max_total_time=30
done
```

Long-running fuzz campaigns run on dedicated infrastructure, not in CI per-PR.

## 11. Chaos testing

### 11.1 Test scenarios

File: `crates/executive/tests/chaos/` (gated behind `#[cfg(feature = "chaos")]`)

| Scenario | Mechanism | Assertions |
|---|---|---|
| Daemon kill -9 mid-turn | Spawn daemon process, send turn request, `kill -9` during LLM call, restart daemon | Session recoverable; items before kill persisted; operation state terminal |
| Daemon kill -9 before response | Kill after tool execution, before response sent | Items persisted; no duplicate execution on restart |
| SQLite WAL corruption | Write garbage bytes into WAL file, then open store | Store opens (or returns explicit error); no silent data loss; existing data intact |
| SQLite disk full | Fill disk during append, attempt writes | Append returns error; store remains consistent; no panic |
| Event spine partial write | Kill process mid-spine-append | Event spine recovers on next open; committed events intact; no partial events |
| Concurrent daemon startups | Two daemon processes attempt to bind same socket | Second daemon gets explicit bind error; first daemon unaffected |
| Memory pressure | Allocate large context, trigger compaction | Compaction succeeds or errors cleanly; no OOM panic |

### 11.2 Feature flag

Add to `crates/executive/Cargo.toml`:

```toml
[features]
chaos = []
```

Chaos tests use:

```rust
#[cfg(feature = "chaos")]
#[tokio::test]
async fn daemon_kill_mid_turn_session_recoverable() {
    // Spawn daemon as subprocess, send RPC, kill, restart, verify
}
```

### 11.3 CI integration

Chaos tests are NOT run in per-PR CI. They run in a nightly or pre-release CI job:

```bash
cargo test --features chaos -- --test-threads=1
```

Single-threaded because chaos tests manage subprocesses and ports.

## 12. Performance benchmarks

### 12.1 Benchmark targets

Located in `crates/executive/benches/`:

| Benchmark | What it measures | Target |
|---|---|---|
| `session_create` | `CanonicalSessionStore::create` latency | Typical hardware: <1ms |
| `session_restore` | `load_session` + `load_items` for 100-item session | Typical hardware: <5ms |
| `turn_execution_mock` | Full `TurnCoordinator::submit_with` with mock LLM | Typical hardware: <10ms overhead (excl. LLM) |
| `tool_execution_path` | MockSandbox::execute latency | Typical hardware: <100us |
| `event_spine_append` | Single event append + projection | Typical hardware: <1ms |
| `concurrent_sessions` | 100 concurrent sessions doing one turn each | Memory: <500MB; no deadlocks |
| `context_projection_build` | ContextAssembler with 200-item history | Typical hardware: <50ms |

### 12.2 Criterion setup

Add to `crates/executive/Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "session_benchmarks"
harness = false

[[bench]]
name = "turn_benchmarks"
harness = false
```

### 12.3 Benchmark example

```rust
// crates/executive/benches/session_benchmarks.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use fabric::*;

fn benchmark_session_create(c: &mut Criterion) {
    c.bench_function("session_create", |b| {
        b.iter(|| {
            let store = CanonicalSessionStore::open(":memory:").unwrap();
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                store.create(SessionRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: SessionId(uuid::Uuid::new_v4().to_string()),
                    parent: None,
                    created_at_ms: 0,
                    status: SessionStatus::Active,
                }).await.unwrap();
            });
        })
    });
}

criterion_group!(benches, benchmark_session_create);
criterion_main!(benches);
```

### 12.4 CI integration

Benchmarks run in a dedicated CI job, not per-PR:

```bash
cargo bench --bench session_benchmarks --bench turn_benchmarks
```

Results are compared against the previous baseline. A regression threshold (e.g., 20% slower) triggers a warning, not a hard failure.

## 13. Implementation phases

### Phase 0 -- Foundation: MockLlmProvider + MockSandbox + TestAletheonBuilder

**Duration**: ~3 days

**Work**:
1. Create `crates/executive/tests/support/mock_llm_provider.rs` with `MockLlmProvider`, `MockTurnSequence`, `MockTurnResponse` types.
2. Create `crates/executive/tests/support/mock_sandbox.rs` with `MockSandbox`, `MockToolResult`, `MockExecutionRecord`.
3. Create `crates/executive/tests/support/test_aletheon_builder.rs` with `TestAletheonBuilder`, `TestAletheon`, `TestInspector`.
4. Add `KernelRuntime::with_clock(clock: Arc<dyn Clock>)` constructor to `crates/kernel/src/runtime.rs` (or equivalent location) so `TestClock` can be injected.
5. Update `crates/executive/tests/support/mod.rs` to export the new support modules.

**Commit message**:
```
test(executive): add MockLlmProvider, MockSandbox, and TestAletheonBuilder

Introduce deterministic test infrastructure for integration testing:
- MockLlmProvider with pre-configured turn sequences and assertion hooks
- MockSandbox with FIFO tool result queues and execution logging
- TestAletheonBuilder that constructs a fully wired test instance
- KernelRuntime::with_clock for deterministic time injection

These types live under tests/support/ to avoid production dependency bloat.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --tests support
```

### Phase 1 -- TurnCoordinator integration tests

**Duration**: ~2 days

**Work**:
1. Create `crates/executive/tests/turn_coordinator_integration.rs` with 13 tests (section 8.1).
2. Each test uses `TestAletheonBuilder` and `MockLlmProvider`.
3. Cover: session creation, item sequencing, operation settlement, cancellation, timeout, concurrency, event spine consistency, context projection, idempotency.

**Commit message**:
```
test(executive): add comprehensive TurnCoordinator integration tests

Add 13 integration tests covering the full TurnCoordinator lifecycle:
session creation, item ordering, operation settlement (success/fail/cancel),
deadline enforcement, concurrent turn isolation, event spine monotonicity,
context projection storage, and idempotent append.

All tests use TestAletheonBuilder with MockLlmProvider for determinism.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --test turn_coordinator_integration
```

### Phase 2 -- EventSourcedSessionStore unit tests

**Duration**: ~1 day

**Work**:
1. Create `crates/executive/tests/event_sourced_store_tests.rs` with 8 tests (section 8.2).
2. Test append-materialize consistency, idempotent replay, projection consistency, poison detection, writer serialization, fork, session creation event.

**Commit message**:
```
test(executive): add EventSourcedSessionStore dedicated unit tests

Add tests for the event-sourced store adapter:
append-materialize round-trip, idempotent replay, projection
consistency, poison detection, writer serialization, fork
through event sourcing, and session creation events.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --test event_sourced_store_tests
```

### Phase 3 -- Daemon turn integration tests

**Duration**: ~2 days

**Work**:
1. Create `crates/executive/tests/daemon_turn_integration.rs` with 7 tests (section 8.3).
2. Tests exercise `DaemonTurnOrchestrator::execute_turn` end-to-end.
3. Create `crates/executive/tests/daemon_react_tests.rs` with 4 tests (section 8.4).

**Commit message**:
```
test(executive): add daemon turn and daemon react integration tests

Add end-to-end tests for DaemonTurnOrchestrator::execute_turn with
mock LLM and mock sandbox. Cover text response, tool call, model error,
kernel error, authenticated turn, operation kind verification, and
context projection extraction.

Add tests for DaemonTurnServices construction, tool execution, seed
messages, and cancellation.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --test daemon_turn_integration
cargo test --package executive --test daemon_react_tests
```

### Phase 4 -- CanonicalSessionStore edge case tests

**Duration**: ~1 day

**Work**:
1. Create `crates/executive/tests/canonical_store_edge_cases.rs` with 7 tests (section 8.5).
2. Cover: schema version rejection, gap detection, concurrent append, fork bounds, filtered load, not-found, corrupt JSON.

**Commit message**:
```
test(executive): add CanonicalSessionStore edge case tests

Add tests for schema version rejection, sequence gap detection,
concurrent append at same sequence, fork bounds, filtered item
loading, session-not-found, and corrupt JSON recovery.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --test canonical_store_edge_cases
```

### Phase 5 -- Snapshot testing infrastructure

**Duration**: ~2 days

**Work**:
1. Add `insta` to `crates/executive/Cargo.toml` and `crates/interact/Cargo.toml` dev-dependencies.
2. Create snapshot tests for: context projections, config schema generation, JSON-RPC protocol messages.
3. Add `cargo insta test` to CI workflow.
4. Create TUI snapshot tests in `crates/interact/tests/snapshots/`.

**Commit message**:
```
test: add insta snapshot testing for context projections and config schema

Add insta-based snapshot tests for deterministic outputs:
- ContextProjectionReceipt JSON snapshots
- ExecutiveConfig JSON Schema generation
- JSON-RPC request/response pairs

CI now verifies snapshots are up-to-date with `cargo insta test`.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo insta test --workspace
```

### Phase 6 -- Fuzzing infrastructure

**Duration**: ~2 days

**Work**:
1. Create `crates/executive/fuzz/` directory with `Cargo.toml`.
2. Implement 7 fuzz targets (section 10.1).
3. Add CI job for quick-check fuzzing (30s per target).
4. Document how to run extended fuzz campaigns locally.

**Commit message**:
```
test: add cargo-fuzz targets for parsing and protocol handling

Add libfuzzer targets for:
- EnvelopeV2 parsing and roundtrip
- JSON-RPC message parsing and method dispatch
- TOML config parsing
- Tool input JSON validation
- Message serialization roundtrip

CI runs 30-second quick-checks per target.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
for target in $(ls crates/executive/fuzz/fuzz_targets/*.rs | xargs -n1 basename | sed 's/\.rs$//'); do
    cargo fuzz run "$target" --fuzz-dir crates/executive/fuzz -- -max_total_time=10 -runs=1000
done
```

### Phase 7 -- Performance benchmarks

**Duration**: ~2 days

**Work**:
1. Add `criterion` to `crates/executive/Cargo.toml` dev-dependencies.
2. Implement 7 benchmark targets (section 12.1).
3. Add CI job for benchmark comparison against baseline.
4. Document benchmark workflow.

**Commit message**:
```
test: add criterion benchmarks for session and turn lifecycle

Add benchmarks for:
- session create/restore latency
- turn execution with mock LLM
- tool execution path
- event spine append
- concurrent sessions (100) resource usage
- context projection build

CI compares against baseline and warns on regressions >20%.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo bench --package executive
```

### Phase 8 -- Chaos testing

**Duration**: ~3 days

**Work**:
1. Add `chaos` feature flag to `crates/executive/Cargo.toml`.
2. Implement 6 chaos test scenarios (section 11.1).
3. Add nightly CI job for chaos tests.
4. Chaos tests spawn real daemon subprocesses and use `kill -9`, file corruption, and disk-full simulation.

**Commit message**:
```
test(executive): add chaos/recovery tests behind 'chaos' feature flag

Add chaos tests for:
- daemon kill -9 mid-turn and session recoverability
- daemon kill before response (no duplicate execution)
- SQLite WAL corruption recovery
- disk-full graceful degradation
- event spine partial write recovery
- concurrent daemon startup isolation

Gated behind #[cfg(feature = "chaos")]; run in nightly CI only.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --package executive --features chaos -- --test-threads=1
```

### Phase 9 -- Known-gap tracking (#[ignore] culture)

**Duration**: <1 day

**Work**:
1. Audit existing tests for flaky behavior (reference: `MEMORY.md` notes flaky `execute_script_hook_inject` test).
2. Mark known-flaky tests with `#[ignore = "reason: tracking issue URL"]`.
3. Add CI step that reports ignored test count.
4. Add developer guideline: `#[ignore]` must include a reason string and tracking issue reference.

**Commit message**:
```
test: establish #[ignore] culture for known-gap tracking

Mark flaky tests with #[ignore = "reason"] and add CI reporting.
Developer guideline: ignored tests must reference a tracking issue.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Verification**:
```bash
cargo test --workspace 2>&1 | grep "ignored"
```

## 14. File structure after all phases

```text
crates/executive/
  tests/
    support/
      mod.rs                                # re-exports test infrastructure
      mock_llm_provider.rs                  # MockLlmProvider, MockTurnSequence
      mock_sandbox.rs                       # MockSandbox, MockExecutionRecord
      test_aletheon_builder.rs              # TestAletheonBuilder, TestAletheon, TestInspector
    turn_coordinator_integration.rs         # 13 tests (Phase 1)
    event_sourced_store_tests.rs            # 8 tests (Phase 2)
    daemon_turn_integration.rs              # 7 tests (Phase 3)
    daemon_react_tests.rs                   # 4 tests (Phase 3)
    canonical_store_edge_cases.rs           # 7 tests (Phase 4)
    snapshots/
      context_projection_snapshot.rs        # insta snapshot tests
      config_schema_snapshot.rs
      jsonrpc_protocol_snapshot.rs
  fuzz/
    Cargo.toml
    fuzz_targets/
      envelope_v2_parse.rs
      envelope_v2_roundtrip.rs
      jsonrpc_message_parse.rs
      jsonrpc_method_dispatch.rs
      toml_config_parse.rs
      tool_input_json.rs
      message_roundtrip.rs
  benches/
    session_benchmarks.rs
    turn_benchmarks.rs
  tests/
    chaos/
      daemon_kill_recovery.rs
      sqlite_corruption.rs
      disk_full.rs
      spine_recovery.rs
      concurrent_startup.rs

crates/interact/
  tests/
    snapshots/
      tui_rendering_snapshot.rs
```

## 15. Cargo.toml additions

### 15.1 `crates/executive/Cargo.toml`

```toml
[dev-dependencies]
# ... existing ...
insta = { version = "1", features = ["json", "toml"] }
criterion = { version = "0.5", features = ["html_reports"] }

[features]
chaos = []

[[bench]]
name = "session_benchmarks"
harness = false

[[bench]]
name = "turn_benchmarks"
harness = false
```

### 15.2 `crates/interact/Cargo.toml`

```toml
[dev-dependencies]
insta = { version = "1", features = ["json"] }
```

## 16. Verification commands

After all phases complete, the following must pass:

```bash
# All existing tests still pass
cargo test --workspace

# New integration tests
cargo test --package executive --test turn_coordinator_integration
cargo test --package executive --test event_sourced_store_tests
cargo test --package executive --test daemon_turn_integration
cargo test --package executive --test daemon_react_tests
cargo test --package executive --test canonical_store_edge_cases

# Snapshot tests
cargo insta test --workspace

# Fuzz quick-check
for target in envelope_v2_parse envelope_v2_roundtrip jsonrpc_message_parse \
              jsonrpc_method_dispatch toml_config_parse tool_input_json \
              message_roundtrip; do
    cargo fuzz run "$target" --fuzz-dir crates/executive/fuzz -- -max_total_time=10 -runs=1000
done

# Benchmarks (no hard failure, but must compile and run)
cargo bench --package executive --no-run

# Chaos tests (nightly only)
cargo test --package executive --features chaos -- --test-threads=1
```

## 17. What not to do

- Do not create a separate `test-support` crate. The mock types are tightly coupled to Executive's internal types and live naturally under `tests/support/`. A separate crate would create circular dependency problems or require publishing internal types.
- Do not mock at the HTTP layer for unit/integration tests. Mock at the `LlmProvider` and `SandboxBackend` trait boundaries. HTTP-level mocking belongs in end-to-end tests only.
- Do not run chaos tests or benchmarks in per-PR CI. They are too slow and too environment-dependent. Nightly CI only.
- Do not snapshot non-deterministic output. Only snapshot outputs that are deterministic given fixed inputs (e.g., config schema, context projection JSON structure).
- Do not fuzz with arbitrary time limits in CI. 30 seconds per target is sufficient for quick-check. Extended campaigns run separately.
- Do not add `#[ignore]` without a reason string and tracking issue reference.

## 18. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `KernelRuntime` lacks clock injection point | Add `with_clock` constructor; minimal change, backward-compatible |
| `TurnCoordinator::new()` hard-codes `:memory:` event spine | Tests use `with_event_spine()` constructor (already exists at `turn_coordinator.rs:74`) |
| Daemon turn tests require full pipeline wiring | Use `TestAletheonBuilder` to wire only the coordinator layer; daemon turn tests wire the full `DaemonTurnOrchestrator` |
| Snapshot churn on first `insta` adoption | Accept all initial snapshots as baseline; subsequent changes are reviewed |
| Chaos tests flaky in CI | Isolate with `--test-threads=1`; add generous timeouts; use unique ports per test |
| MockLlmProvider panic on under-specified sequences | Panic is intentional: the test is buggy if it doesn't configure enough responses. Error message includes index and expected count. |

## 19. Definition of completion

This program is complete when:

1. `TestAletheonBuilder` can construct a fully wired test instance with mock LLM, mock sandbox, and test clock.
2. `MockLlmProvider` records all messages and tool definitions sent to it and returns pre-configured responses.
3. TurnCoordinator has integration tests covering the full lifecycle: create, append, settle, cancel, timeout, concurrency.
4. EventSourcedSessionStore has dedicated unit tests for append-materialize, replay, projection consistency, and poison detection.
5. DaemonTurnOrchestrator::execute_turn has end-to-end integration tests with mock providers.
6. `submit_streaming_daemon_turn` has dedicated tests for the streaming path.
7. Snapshot tests cover context projections, config schema, and JSON-RPC protocol messages.
8. Fuzz targets exist and pass quick-check for all parsing entry points.
9. Criterion benchmarks compile and run for session and turn lifecycle.
10. Chaos tests (nightly CI) cover daemon kill/restart and storage corruption scenarios.
11. `#[ignore]` is used for known-flaky tests with reason strings and tracking issues.
12. CI enforces: existing tests pass, new integration tests pass, snapshots are up-to-date, fuzz quick-checks pass, benchmarks compile.

The practical north star:

```text
Every critical production path has a deterministic integration test.
Every parsing boundary has a fuzz target.
Every user-visible output has a snapshot.
Every lifecycle operation has a benchmark.
Every recovery scenario has a chaos test.
Known gaps are tracked, not hidden.
```
