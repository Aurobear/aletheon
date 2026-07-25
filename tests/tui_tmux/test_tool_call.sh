#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "tool_call — trigger tool, verify result"
tui_start

tui_submit "请使用 Read 工具读取 Cargo.toml 的前 20 行"
# Assert the requested file/tool result, not a model-dependent directory listing.
tui_wait "Cargo.toml|workspace|members" 30 && test_pass "tool_call" || test_fail "tool_call"

tui_stop
test_summary
