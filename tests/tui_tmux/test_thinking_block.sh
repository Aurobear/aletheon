#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "thinking_block — complex question, verify response"
tui_start

tui_submit "用一句话解释什么是递归"
tui_wait "递归|recursion|自己调用|calls itself" 30 && test_pass "thinking_block" || test_fail "thinking_block"

tui_stop
test_summary
