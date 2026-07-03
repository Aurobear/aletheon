# Aletheon Project Issues Audit

**Date:** 2026-06-22
**Branch:** `auro/docs/20260622-project-issues-audit`
**Codebase:** 118,433 lines Rust, 544 `.rs` files, 8 crates
**Tests:** 1,460 pass (excluding cognit), cognit tests fail to compile

---

## Table of Contents

- [P0 — Blocking](#p0--blocking)
  - [1. Default config max_tool_calls=0 breaks all tool use](#1-default-config-max_tool_calls0-breaks-all-tool-use)
  - [2. cognit crate tests fail to compile (12 errors)](#2-cognit-crate-tests-fail-to-compile-12-errors)
  - [3. Four unimplemented!() in production code paths](#3-four-unimplemented-in-production-code-paths)
- [P1 — Critical](#p1--critical)
  - [4. Exec mode is a bare shell missing all subsystems](#4-exec-mode-is-a-bare-shell-missing-all-subsystems)
  - [5. Default socket path /run/ requires root](#5-default-socket-path-run-requires-root)
  - [6. 17x .lock().unwrap() will cascade-panic on mutex poison](#6-17x-lockunwrap-will-cascade-panic-on-mutex-poison)
  - [7. ChannelEventSink silently drops events when full](#7-channeleventsink-silently-drops-events-when-full)
  - [8. API key resolution silently returns empty string](#8-api-key-resolution-silently-returns-empty-string)
  - [9. Transport auto-detection misidentifies Anthropic URLs](#9-transport-auto-detection-misidentifies-anthropic-urls)
  - [10. ToolBudget and reflection engine have conflicting limits](#10-toolbudget-and-reflection-engine-have-conflicting-limits)
- [P2 — Design](#p2--design)
  - [11. TUI App struct is a 34-field god object](#11-tui-app-struct-is-a-34-field-god-object)
  - [12. Two parallel hook systems with no interaction](#12-two-parallel-hook-systems-with-no-interaction)
  - [13. SandboxFirst verdict silently ignored](#13-sandboxfirst-verdict-silently-ignored)
  - [14. Dual compaction in ReActLoop and SessionManager](#14-dual-compaction-in-reactloop-and-sessionmanager)
  - [15. Hook loader uses hand-rolled TOML parser](#15-hook-loader-uses-hand-rolled-toml-parser)
  - [16. MCP embedded server uses hardcoded session ID](#16-mcp-embedded-server-uses-hardcoded-session-id)
  - [17. Hook system discards stderr from hook scripts](#17-hook-system-discards-stderr-from-hook-scripts)
- [P3 — Quality](#p3--quality)
  - [18. 183 compiler warnings across workspace](#18-183-compiler-warnings-across-workspace)
  - [19. LanceDB vector store is entirely stubbed](#19-lancedb-vector-store-is-entirely-stubbed)
  - [20. Controller scaffold never wired in (dead code)](#20-controller-scaffold-never-wired-in-dead-code)
  - [21. Dasein perception sources are dead code](#21-dasein-perception-sources-are-dead-code)
  - [22. partial_cmp().unwrap() on floats will panic on NaN](#22-partial_cmpunwrap-on-floats-will-panic-on-nan)
  - [23. Release workflow references wrong package names](#23-release-workflow-references-wrong-package-names)
  - [24. ModelRouter tests skip reasoning task classification](#24-modelrouter-tests-skip-reasoning-task-classification)
  - [25. Exec mode silently falls back to /tmp on path failure](#25-exec-mode-silently-falls-back-to-tmp-on-path-failure)
  - [26. IPC socket fallbacks use /tmp unsafely](#26-ipc-socket-fallbacks-use-tmp-unsafely)
- [Summary Table](#summary-table)

---

## P0 — Blocking

### 1. Default config max_tool_calls=0 breaks all tool use

**File:** `config/default.toml:101`, `crates/runtime/src/core/react_loop/tool_budget.rs:65-74`

**Problem:** The config comment says `# Maximum tool calls per turn (0 = unlimited)`, but the implementation does NOT treat 0 as unlimited. `ToolBudget::can_call()` checks `self.used_calls < self.max_calls`, so `0 < 0 = false` — every tool call is rejected immediately on the first attempt.

**Impact:** Out-of-the-box configuration is completely non-functional for tool use.

**Fix:** Either change the default to a reasonable limit (e.g., 50), or change `can_call()` to treat 0 as unlimited:
```rust
pub fn can_call(&self) -> bool {
    self.max_calls == 0 || self.used_calls < self.max_calls
}
```

---

### 2. cognit crate tests fail to compile (12 errors)

**File:** `crates/cognit/src/core/tests.rs`

**Problem:** 12 compilation errors prevent `cargo test --workspace`:
- `ProviderConfig` struct initializers missing the `max_context_length` field (6 occurrences)
- Missing `#[async_trait]` attribute in scope
- `Observation` struct not found (3 occurrences)
- Lifetime parameter mismatches on `complete` and `complete_stream` trait methods (2 occurrences)

**Impact:** CI `cargo test` step may fail. Cannot verify cognit crate correctness.

**Fix:** Update test initializers to include `max_context_length`, add missing imports.

---

### 3. Four unimplemented!() in production code paths

**Files:**
- `crates/corpus/src/tools/tools/executor.rs:369`
- `crates/runtime/src/impl/memory/auto_memory.rs:291,321`
- `crates/runtime/src/impl/memory/compressor/mod.rs:161`

**Problem:** These are not test-only stubs — they sit in production code paths. Reaching them causes an immediate panic with no recovery.

**Impact:** Runtime crash if these code paths are hit.

**Fix:** Replace with `anyhow::bail!()` or proper error returns, or implement the missing logic.

---

## P1 — Critical

### 4. Exec mode is a bare shell missing all subsystems

**File:** `crates/runtime/src/bin/aletheon-exec.rs:194-278`

**Problem:** The `aletheon-exec` binary has its own raw agent loop that lacks every subsystem the daemon's `handle_chat()` integrates:

| Subsystem | Daemon | Exec |
|-----------|--------|------|
| ToolBudget / CircuitBreaker | ✅ | ❌ |
| Hooks (pre_turn, post_tool) | ✅ | ❌ |
| Memory (FactStore, Recall, Core, Auto, Episodic) | ✅ | ❌ |
| SelfField safety review | ✅ | ❌ |
| Model routing | ✅ | ❌ |
| Skill router | ✅ | ❌ |
| Reflection / evolution | ✅ | ❌ |
| Storm breaker | ✅ | ❌ |
| Approval gate wiring | ✅ | ❌ (created but unused) |
| Streaming output | ✅ | ❌ |
| Session persistence | ✅ | ❌ |

**Impact:** Exec mode is a chatbot wrapper, not an agent. Users who try `aletheon-exec` get a completely different (and broken) experience.

**Fix:** Refactor exec mode to reuse the daemon's `handle_chat()` pipeline, or clearly document it as a minimal testing tool.

---

### 5. Default socket path /run/ requires root

**Files:**
- `config/default.toml:87`
- `crates/cognit/src/config/mod.rs:256`
- `crates/runtime/src/core/config/infra.rs:109`
- `crates/interact/src/tui/cli.rs:17`
- `crates/base/src/types/paths.rs:11`

**Problem:** Default socket is `/run/aletheond/aletheond.sock`. The `/run/` directory requires root/systemd access. A separate constant `SOCKET_DIR` uses `/var/run/aletheon` which has the same problem.

**Impact:** Non-root users cannot start or connect to the daemon without modifying config.

**Fix:** Default to `~/.aletheon/aletheond.sock` for user-mode, keep `/run/` only for the systemd service unit.

---

### 6. 17x .lock().unwrap() will cascade-panic on mutex poison

**Files (most critical):**
- `crates/base/src/types/resource.rs:94,108,130` — `ManagedResource` (used everywhere)
- `crates/cognit/src/bridge/learning.rs:52,61,67` — `LearningBridge`
- `crates/base/src/ipc/bus/in_process.rs:86,96` — `PriorityChannel`
- `crates/dasein/src/dasein/sorge.rs:49`

**Problem:** If any thread panics while holding a mutex, the mutex becomes "poisoned". Every subsequent `.lock().unwrap()` on that mutex will also panic, cascading through the runtime.

**Impact:** A single panic in one subsystem can crash the entire daemon.

**Fix:** Use `.lock().unwrap_or_else(|e| e.into_inner())` for non-critical paths, or implement poison recovery with logging.

---

### 7. ChannelEventSink silently drops events when full

**File:** `crates/runtime/src/core/event_sink.rs:174`

```rust
fn emit(&self, event: Event) {
    let _ = self.tx.try_send(event);
}
```

**Problem:** Channel capacity is 64. When full, events are silently dropped — no logging, no backpressure, no error signal. Critical events like `BudgetExceeded`, `CircuitBreakerTripped`, and `TurnDone` can be lost.

**Impact:** TUI can get stuck in "streaming" mode forever if `TurnDone` is dropped. Safety events may never reach the user.

**Fix:** At minimum, log dropped events. Better: use `send().await` with a timeout, or increase channel capacity and add overflow detection.

---

### 8. API key resolution silently returns empty string

**File:** `crates/cognit/src/impl/provider_registry.rs:159-165`

```rust
fn resolve_api_key(&self, config: &ProviderConfig) -> String {
    if !config.api_key.is_empty() {
        return config.api_key.clone();
    }
    let env_name = format!("{}_API_KEY", config.name.to_uppercase().replace('-', "_"));
    std::env::var(&env_name).unwrap_or_default()
}
```

**Problem:** If neither config file nor environment variable has the API key, returns `""` with no warning. The provider is created with an empty key; the error surfaces only on the first API call as a cryptic 401.

**Impact:** Users get confusing auth errors instead of a clear "API key not configured" message at startup.

**Fix:** Return `Result<String>` and fail fast at provider creation time if key is missing.

---

### 9. Transport auto-detection misidentifies Anthropic URLs

**File:** `crates/cognit/src/impl/provider_registry.rs:20`

**Problem:** `detect_transport()` checks if URL ends with `/anthropic`. So:
- `https://api.anthropic.com` → detected as **OpenAI** (wrong)
- `https://proxy.example.com/anthropic` → detected as Anthropic (correct)

Users must explicitly set `transport = "anthropic"` or use a URL ending in `/anthropic`.

**Impact:** Silent misconfiguration leads to protocol errors that are hard to diagnose.

**Fix:** Check for `anthropic` in the host portion of the URL, or require explicit transport configuration.

---

### 10. ToolBudget and reflection engine have conflicting limits

**Files:**
- `crates/runtime/src/core/react_loop/tool_budget.rs` — configurable `max_tool_calls`
- `crates/runtime/src/core/react_loop/reflection.rs:99-104` — hardcoded limit of 10

**Problem:** The reflection engine hard-stops at 10 tool calls regardless of the configured `max_tool_calls`. If budget is set to 25, reflection still kills the loop at 10.

```rust
} else if context.tool_calls_made >= 10 {
    self.should_stop = true;
    ReflectionRecommendation::Stop(TerminationReason::BudgetExhausted)
```

**Impact:** Configured budget above 10 is effectively ignored by reflection.

**Fix:** Pass `max_tool_calls` into the reflection engine, or remove the hard limit and rely solely on ToolBudget.

---

## P2 — Design

### 11. TUI App struct is a 34-field god object

**File:** `crates/interact/src/tui/mod.rs:207-261`

**Problem:** The `App` struct has 34 fields covering input handling, streaming state, approval flow, completion, pager, sub-agents, token tracking, and more. This is a textbook god object — changing any feature requires understanding the entire struct.

**Fields include:** `chat`, `input_buf`, `cursor`, `stream`, `read_buf`, `running`, `streaming`, `turn_active`, `response_buf`, `caps`, `skill_loader`, `model_name`, `status`, `last_ctrl_c`, `has_cjk`, `pending_submit`, `first_render`, `pending_approval`, `stream_ctrl`, `active_tools`, `turn_tokens`, `total_tokens`, `history`, `completion`, `pager`, `frame_counter`, `app_state`, `plan_view`, `sub_agents`, `current_iteration`...

**Fix:** Extract focused components: `InputState`, `StreamManager`, `ApprovalFlow`, `TokenTracker`, `CompletionEngine`.

---

### 12. Two parallel hook systems with no interaction

**Files:**
- `crates/runtime/src/impl/daemon/handler/mod.rs:820` — `run_hook_scripts()` (TOML-configured scripts)
- `crates/runtime/src/impl/hooks/registry.rs:75` — `HookRegistry::execute()` (~/.aletheon/hooks/*.toml)

**Problem:** Two completely separate hook execution paths are called in `handle_chat()`:
- Script hooks inject output into `effective_message` as `[Hook output]`
- Registry hooks use `HookResult::Block` / `HookResult::Inject`

A registry hook cannot block a script hook, and vice versa. The `run_hook_scripts` function also lacks the structured JSON output parsing that `parse_hook_output()` provides.

**Fix:** Unify into a single hook pipeline with a clear priority/blocking chain.

---

### 13. SandboxFirst verdict silently ignored

**File:** `crates/runtime/src/core/orchestrator.rs:327-334`

**Problem:** The comment says "Sandbox infrastructure exists but is complex to wire here. Log and proceed without sandbox for now." The `SandboxFirst` verdict from SelfField is treated as a no-op.

**Impact:** The security sandbox is advertised in the architecture but has no effect at runtime.

**Fix:** Either wire the sandbox or remove `SandboxFirst` from the verdict enum to avoid a false sense of security.

---

### 14. Dual compaction in ReActLoop and SessionManager

**Files:**
- `crates/runtime/src/core/react_loop/tool_exec.rs:359-361` — `self.compressor.maybe_compact()`
- `crates/runtime/src/impl/daemon/handler/chat.rs:674` — `sm.compact_if_needed()`

**Problem:** Two different compressor instances with potentially different thresholds run compaction. The ReActLoop compressor uses `tail_token_budget` and `context_window_tokens`; the SessionManager compressor may have different settings.

**Impact:** Double-compaction (wasted tokens) or inconsistent behavior depending on which compressor fires first.

**Fix:** Consolidate into a single compaction point, or have the SessionManager skip if ReActLoop already compacted.

---

### 15. Hook loader uses hand-rolled TOML parser

**File:** `crates/runtime/src/impl/hooks/loader.rs:84-142`

**Problem:** `load_hook_file()` uses a manual line-by-line parser (`parse_toml_kv` at line 144) instead of the `toml` crate used elsewhere. This parser:
- Does not handle comments after values (`name = "foo" # comment`)
- Does not handle multi-line strings
- Does not handle quoted values with embedded quotes
- Only parses `[hook]` section, silently ignoring all others

**Impact:** Valid TOML files will fail to parse or parse incorrectly.

**Fix:** Use the `toml` crate consistently.

---

### 16. MCP embedded server uses hardcoded session ID

**File:** `crates/runtime/src/impl/daemon/mcp_embedded.rs:167`

```rust
session_id: "mcp-session".into(),
```

**Problem:** Every MCP tool call shares the same session ID. Audit logs cannot distinguish between different MCP clients. Session-scoped approvals or state are shared across all callers.

**Fix:** Generate a unique session ID per MCP connection.

---

### 17. Hook system discards stderr from hook scripts

**File:** `crates/runtime/src/impl/daemon/handler/mod.rs:831`

```rust
.stderr(std::process::Stdio::null())
```

**Problem:** Hook script stderr is discarded. When a hook fails with a non-zero exit code, only the exit code is logged — not the error message.

**Impact:** Debugging hook failures requires modifying the hook script to redirect stderr to stdout.

**Fix:** Capture stderr and include it in the failure log message.

---

## P3 — Quality

### 18. 183 compiler warnings across workspace

**Problem:** Significant dead code, unused imports, deprecated API usage. The `runtime` crate alone has 44 warnings.

**Impact:** Signal-to-noise ratio makes it hard to spot real issues. Suggests rapid development without cleanup passes.

**Fix:** Dedicate a cleanup pass: remove dead code, fix unused imports, address deprecations.

---

### 19. LanceDB vector store is entirely stubbed

**File:** `crates/runtime/src/impl/memory/vector_store.rs:236-258`

**Problem:** All methods (`upsert`, `search`, `delete`, `count`) bail with "not yet fully implemented". `VectorBackend::Lance` and `VectorBackend::Auto` are non-functional.

**Fix:** Implement or remove the Lance backend from the `Auto` selection logic.

---

### 20. Controller scaffold never wired in (dead code)

**File:** `crates/runtime/src/core/controller.rs:7-10,48`

**Problem:** The module doc says "Scaffold module that will be wired into TUI and HTTP frontends in a future phase." The entire `Controller` struct is `#[allow(dead_code)]`.

**Fix:** Either implement and wire in, or remove the scaffold.

---

### 21. Dasein perception sources are dead code

**Files:** `crates/dasein/src/impl/perception/sources/` — `journald_source.rs`, `inotify_source.rs`, `proc_source.rs`, `bottleneck_detector.rs`

**Problem:** All fields are `#[allow(dead_code)]`. The perception sources exist but are never connected to the perception pipeline.

**Fix:** Wire in or remove.

---

### 22. partial_cmp().unwrap() on floats will panic on NaN

**File:** `crates/cognit/src/core/learner.rs:189`

```rust
.min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
```

**Problem:** `partial_cmp` on `f64` returns `None` for NaN. If a confidence value is NaN, this panics.

**Fix:** Use `.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)` or filter out NaN values.

---

### 23. Release workflow references wrong package names

**File:** `.github/workflows/release.yml`

**Problem:** Builds `-p aletheond -p aletheon-exec -p interact`, but:
- The daemon binary is in the `runtime` crate (package name `runtime`), not `aletheond`
- The CLI binary is in the `interact` crate (package name `interact`), not `interact`

**Impact:** Release builds will fail.

**Fix:** Use the correct Cargo package names.

---

### 24. ModelRouter tests skip reasoning task classification

**File:** `crates/runtime/src/impl/daemon/model_router.rs:177-228`

**Problem:** Tests call `ModelRouter::classify_message_static()` (test-only static version at line 220) which does NOT call `is_reasoning_task()`. The production `classify_message()` (line 69) does call it, but reasoning classification is completely untested.

**Fix:** Add test cases that exercise `is_reasoning_task()` through the production code path.

---

### 25. Exec mode silently falls back to /tmp on path failure

**File:** `crates/runtime/src/bin/aletheon-exec.rs:138-139`

```rust
.canonicalize()
.unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));
```

**Problem:** If the working directory cannot be resolved, exec mode silently falls back to `/tmp`. Tools will execute in an unexpected directory with no warning.

**Fix:** Return an error instead of silently falling back.

---

### 26. IPC socket fallbacks use /tmp unsafely

**Files:**
- `crates/base/src/ipc/backends/manager.rs:120,178,191,208,254`
- `crates/base/src/ipc/backends/unix_socket.rs:19`
- `crates/base/src/ipc/backends/unix_socket_transport.rs:26`

**Problem:** Multiple fallback paths use `PathBuf::from("/tmp")` and the constant `DEFAULT_SOCKET_DIR = "/tmp/agent-ipc"`. On multi-user systems, `/tmp` is world-readable and susceptible to symlink attacks.

**Fix:** Use `tempfile::tempdir()` or XDG runtime directory (`$XDG_RUNTIME_DIR`).

---

## Summary Table

| # | Severity | Issue | File(s) | Fix Effort |
|---|----------|-------|---------|------------|
| 1 | **P0** | max_tool_calls=0 breaks tool use | tool_budget.rs | 5min |
| 2 | **P0** | cognit tests won't compile | cognit/src/core/tests.rs | 30min |
| 3 | **P0** | 4x unimplemented!() in prod code | executor, auto_memory, compressor | 1-2h |
| 4 | **P1** | Exec mode missing all subsystems | aletheon-exec.rs | 1-2d |
| 5 | **P1** | Socket path requires root | multiple | 30min |
| 6 | **P1** | 17x .lock().unwrap() cascade risk | multiple | 2h |
| 7 | **P1** | Event channel drops silently | event_sink.rs | 30min |
| 8 | **P1** | API key silent empty string | provider_registry.rs | 1h |
| 9 | **P1** | Transport detection wrong for Anthropic | provider_registry.rs | 30min |
| 10 | **P1** | ToolBudget vs reflection limit conflict | reflection.rs | 30min |
| 11 | P2 | TUI App 34-field god object | tui/mod.rs | 2-3d |
| 12 | P2 | Two hook systems don't interact | handler/mod.rs, registry.rs | 1d |
| 13 | P2 | SandboxFirst silently ignored | orchestrator.rs | 1d |
| 14 | P2 | Dual compaction | tool_exec.rs, chat.rs | 2h |
| 15 | P2 | Hand-rolled TOML parser | hooks/loader.rs | 1h |
| 16 | P2 | Hardcoded MCP session ID | mcp_embedded.rs | 30min |
| 17 | P2 | Hook stderr discarded | handler/mod.rs | 30min |
| 18 | P3 | 183 compiler warnings | workspace-wide | 2h |
| 19 | P3 | LanceDB vector store stubs | vector_store.rs | 1d |
| 20 | P3 | Controller dead code | controller.rs | 1h |
| 21 | P3 | Dasein perception dead code | perception/sources/ | 1h |
| 22 | P3 | partial_cmp().unwrap() on NaN | learner.rs | 5min |
| 23 | P3 | Release workflow wrong package names | release.yml | 15min |
| 24 | P3 | ModelRouter tests skip reasoning | model_router.rs | 30min |
| 25 | P3 | Exec mode /tmp fallback | aletheon-exec.rs | 10min |
| 26 | P3 | IPC /tmp symlink risk | ipc/backends/ | 1h |

**Total: 26 issues** — 3 P0, 7 P1, 7 P2, 9 P3
