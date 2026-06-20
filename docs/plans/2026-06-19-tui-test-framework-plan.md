# TUI Test Framework — Implementation Plan

**Design doc**: `docs/plans/2026-06-19-tui-test-framework-design.md`
**Date**: 2026-06-19

## Tasks

### Task 1: Fix model naming in config.toml ✅ DONE
- File: `~/.aletheon/config.toml`
- Changed `deepseek/deepseek-v4-flash` → `deepseek-v4-flash` in model_routing
- Changed `deepseek/deepseek-v4-pro[1m]` → `deepseek-v4-pro[1m]` in model_routing + aliases

### Task 2: Add CLI flags to aletheon-cli
- File: `crates/binaries/aletheon-cli/src/main.rs`
- Add clap args: `--test-input`, `--record-frames`, `--record-events`, `--auto-submit`, `--test-timeout`
- Pass config to App constructor

### Task 3: Add FrameRecorder to TUI
- File: `crates/aletheon-body/src/impl/ui/mod.rs`
- New struct `FrameRecorder` with `write(snapshot)` method
- `FrameSnapshot` struct: ts, cols, rows, content, thinking_visible, tool_count
- Wire into render loop: after `f.render_widget(...)`, call `frame_recorder.write(...)`

### Task 4: Add EventRecorder to TUI
- File: `crates/aletheon-body/src/impl/ui/mod.rs`
- New struct `EventRecorder` with `write(event_json)` method
- Wrap `handle_event()` to log before processing

### Task 5: Add TestInputReader
- File: `crates/aletheon-body/src/impl/ui/input.rs`
- New struct `TestInputReader` that reads lines from file
- When `--auto-submit` flag is set, replace stdin input with file lines
- After each response completes (turn_done event), submit next line

### Task 6: Create test scenario files
- Directory: `tests/tui_scenarios/`
- 7 files: `basic_response.txt`, `tool_call.txt`, `thinking_block.txt`, `rapid_fire.txt`, `long_output.txt`, `error_recovery.txt`, `ctrl_shortcuts.txt`
- Each file is just input text, one line per scenario step

### Task 7: Create orchestrator script
- File: `scripts/test_tui_integration.sh`
- Start daemon with test config
- Run TUI with test flags for each scenario
- Collect artifacts to `/tmp/aletheon-tui-test-<ts>/`

### Task 8: Create verification script
- File: `scripts/verify_tui_test.sh`
- Read frames.jsonl, events.jsonl, daemon.log
- Run assertions: event sequence, widget presence, no panic
- Generate summary report

## Dependencies

```
Task 2 (CLI flags) ──┐
Task 3 (FrameRecorder) ├── Task 5 (TestInputReader) ── Task 7 (orchestrator) ── Task 8 (verifier)
Task 4 (EventRecorder) ┘
Task 6 (scenarios) ─────────────────────────────────────────┘
```

## Validation

After all tasks:
1. `cargo build --release` — compiles without errors
2. `cargo test` — existing tests pass
3. `./scripts/test_tui_integration.sh` — all 7 scenarios run
4. `./scripts/verify_tui_test.sh /tmp/aletheon-tui-test-*/` — all assertions pass
