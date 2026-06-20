#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "error_recovery — invalid file read, verify graceful handling"
tui_start

tui_submit "读取文件 /tmp/nonexistent_xyz_999.txt 的内容"
# The LLM may respond in various ways — just check that some response appears
tui_wait "reflect|不存在|error|无法|没有|Error|No such" 30 && test_pass "error_recovery" || test_fail "error_recovery"

tui_stop
test_summary
