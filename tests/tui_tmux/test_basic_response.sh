#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "basic_response — send hello, get response"
tui_start

tui_submit "hello"
tui_wait "Hello|Hey|你好|help" 30 && test_pass "basic_response" || test_fail "basic_response"

tui_stop
test_summary
