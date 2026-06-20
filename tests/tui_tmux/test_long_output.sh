#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "long_output — code generation, verify response"
tui_start

tui_submit "用Python写一个hello world"
tui_wait "print|hello|Hello|Python|世界" 30 && test_pass "long_output" || test_fail "long_output"

tui_stop
test_summary
