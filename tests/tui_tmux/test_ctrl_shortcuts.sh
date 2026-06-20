#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "ctrl_shortcuts — verify basic key handling"
tui_start

# Type something, then clear with Esc
tui_send "hello world"
sleep 0.5
tui_key Escape
sleep 0.5
# After Esc, input should be cleared
# Type a new message and submit
tui_submit "hi"
tui_wait "hi" 10

test_pass "ctrl_shortcuts"

tui_stop
test_summary
