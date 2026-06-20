#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "interrupt — Ctrl+C cancels long-running request"
tui_start

tui_submit "Write a very detailed 1000 word essay about the history of computer science"
sleep 3
tui_key "C-c"
tui_wait "Interrupt|interrupt|cancel|Cancel" 10 && test_pass "interrupt" || test_fail "interrupt"

tui_stop
test_summary
