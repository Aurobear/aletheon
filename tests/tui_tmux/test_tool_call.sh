#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "tool_call — trigger tool, verify result"
tui_start

tui_submit "列出当前目录的文件"
# The tool result appears as a system message or inline — check for common files
tui_wait "Cargo|crates|reflect|src" 30 && test_pass "tool_call" || test_fail "tool_call"

tui_stop
test_summary
