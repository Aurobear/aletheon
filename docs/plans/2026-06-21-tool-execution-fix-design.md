# Tool Execution Bug Fix Design

**Date:** 2026-06-21
**Scope:** Minimal fixes to make tool execution work in Aletheon

## Problem

Tool execution is completely broken. When the LLM needs to call tools (e.g., `bash_exec` for `ls`), the system either:
1. Routes to a cheap model that doesn't support tool calling → empty response
2. Tool results lack `call_id` → TUI tool cards never complete
3. bash_exec has no timeout → commands can hang forever
4. Duplicate event emission → race conditions

## Fixes

### Fix 1: ModelRouter Fallback (CRITICAL)

**File:** `crates/runtime/src/impl/daemon/handler/chat.rs`

When `model_router.create_provider(task_type)` returns a provider that produces an empty ReAct loop result (`len=0`), retry with the `General` (default) model.

**Implementation:**
- After `react_loop.run_streaming()` returns, check if `text.is_empty()` and no tools were executed
- If so, log a warning and retry with `TaskType::General` model
- Cap retries at 1 to avoid loops

### Fix 2: ToolResult call_id (HIGH)

**Files:** `crates/runtime/src/core/event_sink.rs`, `crates/runtime/src/core/react_loop/tool_exec.rs`, `crates/runtime/src/impl/daemon/handler/format.rs`

Add `call_id` to `Event::ToolResult` so the TUI can match results to tool cards.

**Implementation:**
- Add `call_id: String` field to `Event::ToolResult`
- In `tool_exec.rs`, pass the tool call id when emitting `ToolResult`
- In `format.rs`, include `call_id` in the JSON output

### Fix 3: bash_exec timeout (MEDIUM)

**File:** `crates/corpus/src/tools/tools/bash_exec.rs`

The `timeout_seconds` parameter is read but not applied (`_timeout`).

**Implementation:**
- Use `tokio::time::timeout(Duration::from_secs(timeout), command.output())` 
- Return timeout error if exceeded

### Fix 4: Remove duplicate tool_call_result emission (MEDIUM)

**File:** `crates/runtime/src/impl/daemon/handler/chat.rs`

The `execute_tool` closure emits `tool_call_result` directly via `notify_tx` AND the EventSink also emits `ToolResult` which gets converted to another `tool_call_result`. Remove the manual one in chat.rs.

**Implementation:**
- Remove lines 412-422 in chat.rs (the `notify_tx_arc` manual emission)
- The EventSink path (with call_id from Fix 2) is the single source of truth

## Files Changed

| File | Change |
|------|--------|
| `crates/runtime/src/impl/daemon/handler/chat.rs` | Fix 1 (fallback), Fix 4 (remove dup emission) |
| `crates/runtime/src/core/event_sink.rs` | Fix 2 (add call_id to ToolResult) |
| `crates/runtime/src/core/react_loop/tool_exec.rs` | Fix 2 (pass call_id) |
| `crates/runtime/src/impl/daemon/handler/format.rs` | Fix 2 (include call_id in JSON) |
| `crates/corpus/src/tools/tools/bash_exec.rs` | Fix 3 (implement timeout) |

## Validation

1. `cargo build --release` — compiles
2. `aletheon -m "list files in current directory"` — should execute `ls` and show results
3. `aletheon -m "say hello"` — should still work (no regression)
4. `aletheon -m "run sleep 60" --timeout 5` — should timeout
