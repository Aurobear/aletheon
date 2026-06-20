#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "mode_switch — switch between plan and default modes"
tui_start

tui_submit "/mode plan"
tui_wait "plan|Plan|📋" 5 && test_pass "mode_switch_plan" || test_fail "mode_switch_plan"

tui_submit "/mode default"
tui_wait "default|Default|💬" 5 && test_pass "mode_switch_default" || test_fail "mode_switch_default"

tui_stop
test_summary
