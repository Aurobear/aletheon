# Testing Strategy

Aletheon's testing approach covers six layers: unit tests, integration tests, sandbox tests, eBPF tests, end-to-end tests, and performance benchmarks. This document explains each layer, how to run tests, and how to write new ones.

---

## Test Layers

| Layer | What it tests | Tool | Coverage target | Failure tolerance |
|-------|--------------|------|-----------------|-------------------|
| Unit | Pure logic (parsers, validators, classifiers) | `cargo test` | >80% line coverage | Not acceptable |
| Integration | Module interactions (engine+tool, perception+bridge) | `cargo test --test` | Core paths 100% | Not acceptable |
| Sandbox | Isolation works (namespace, seccomp, cgroups) | bubblewrap + test | All sandbox backends | Acceptable (env-dependent) |
| eBPF | Kernel program loading and event collection | libbpf + test | Load + 3 event types | Acceptable (kernel-version) |
| End-to-end | Full user flows | cli + test | 5 critical scenarios | Not acceptable |
| Performance | Latency/throughput | criterion | Baseline comparison | Acceptable |

---

## Running Tests

### All Tests

```bash
cargo test --workspace
```

This runs all unit and integration tests across every crate. Currently 600+ tests pass.

### Specific Crate

```bash
cargo test -p base
cargo test -p dasein
cargo test -p runtime
```

### Specific Test

```bash
cargo test test_reflection_entry_roundtrip
cargo test test_loop_detector
```

### With Logging

```bash
RUST_LOG=debug cargo test
RUST_LOG=aletheon_body=trace cargo test -p corpus
```

### Single Test with Output

```bash
cargo test test_name -- --nocapture
```

---

## Writing Tests

### Naming Convention

```rust
#[test]
fn test_<function>_with_<input>_should_<expected>() {
    // Arrange
    let input = setup_test_data();

    // Act
    let result = function_under_test(input);

    // Assert
    assert_eq!(result, expected_value);
}
```

Use the pattern `test_<what>_<condition>_<expected>` so test names read as specifications.

### Test Structure

Every test follows Arrange-Act-Assert:

```rust
#[test]
fn test_policy_engine_deny_destructive_operation() {
    // Arrange
    let engine = PolicyEngine::new(strict_config());
    let op = Operation::FileDelete { path: "/etc/passwd" };

    // Act
    let decision = engine.evaluate(&op);

    // Assert
    assert_eq!(decision, Decision::Deny);
}
```

### Testing with Mocks

Each crate has a `testing/` module with mock implementations:

| Mock | Location | Replaces |
|------|----------|----------|
| `MockLlm` | `crates/cognit/src/testing/` | Real LLM provider |
| `MockSandbox` | `crates/corpus/src/testing/` | bubblewrap sandbox |
| `MockMemory` | `crates/memory/src/testing/` | SQLite memory backend |
| `MockPerception` | `crates/dasein/src/testing/` | /proc, journald sources |
| `MockStateProvider` | `crates/dasein/src/impl/perception/fuse/provider.rs` | Live system state |

Example using `MockLlm`:

```rust
#[tokio::test]
async fn test_engine_handles_tool_call() {
    let llm = MockLlm::new()
        .with_response("I'll list the files.")
        .with_tool_call("bash", json!({"command": "ls"}))
        .with_response("Here are the files.");

    let engine = Engine::new(llm, mock_tools(), mock_memory());
    let result = engine.run_turn("list files").await;

    assert!(result.contains("files"));
}
```

---

## End-to-End Scenarios

Five critical scenarios must always pass:

| Scenario | Steps | Verification |
|----------|-------|-------------|
| Basic conversation | Start daemon -> cli sends message -> receive response | Response is non-empty, no errors |
| Tool call | Request "list files" -> agent calls `bash_exec("ls")` -> returns file list | Tool executes, result is correct |
| Memory persistence | Tell agent a preference -> restart daemon -> converse again -> agent remembers | CoreMemory restores correctly |
| Security block | Request "rm -rf /" -> agent blocks -> returns safety message | L3 operation is denied |
| Crash recovery | Kill daemon with SIGKILL -> systemd restarts -> session resumes | Session data is not lost |

These scenarios are tested in CI and must pass before any merge.

---

## Test Coverage

### Install Coverage Tool

```bash
cargo install cargo-tarpaulin
```

### Generate Report

```bash
cargo tarpaulin --workspace --out Html
```

### Coverage Targets

| Area | Target |
|------|--------|
| Core modules (abi, brain, self, runtime) | 80%+ |
| Tool modules (body) | 70%+ |
| Auxiliary modules (comm, meta) | 60%+ |

---

## Continuous Integration

GitHub Actions runs on every push and PR:

1. `cargo fmt --all -- --check` -- format check
2. `cargo clippy --workspace -- -D warnings` -- static analysis
3. `cargo test --workspace` -- full test suite
4. `cargo doc --workspace --no-deps` -- documentation build (warnings are errors)

All four must pass before merge. See [CI Pipeline](../design/testing/ci-pipeline.md) for the workflow configuration.

---

## Performance Testing

### Benchmarks

```bash
cargo bench
```

Uses [criterion](https://github.com/bheisler/criterion.rs) for statistically rigorous benchmarks.

### Flame Graphs

```bash
cargo install cargo-flamegraph
cargo flamegraph --bin daemon
```

### Key Metrics

| Metric | Target |
|--------|--------|
| ReAct loop overhead (no LLM) | < 5ms |
| Tool dispatch latency | < 1ms |
| Memory lookup (L1) | < 0.1ms |
| Memory lookup (L2 SQLite) | < 5ms |
| Perception event processing | < 10ms |

---

## Debugging Tests

### Run with Backtrace

```bash
RUST_BACKTRACE=1 cargo test test_failing_test
```

### Run with Debugger

```bash
cargo test test_name -- --nocapture
# Attach gdb/lldb to the test process
```

### Isolate Flaky Tests

```bash
# Run a single test 100 times to detect flakiness
for i in $(seq 1 100); do
  cargo test test_flaky || break
done
```

---

## Mock Strategy

See [Mock Strategy](../design/testing/mock-strategy.md) for the full mock architecture, including:
- How mocks are structured per crate
- Recording and replaying mock interactions
- Testing error paths and edge cases

---

## Related Documents

- [Test Strategy](../design/testing/test-strategy.md) -- detailed test layer definitions and security test cases
- [Mock Strategy](../design/testing/mock-strategy.md) -- mock infrastructure design
- [CI Pipeline](../design/testing/ci-pipeline.md) -- GitHub Actions workflow configuration
- [Contributing Guide](../../CONTRIBUTING.md) -- how to submit changes
