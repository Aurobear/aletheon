#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "plan_mode — enter plan mode and get a plan response"
tui_start

tui_submit "/mode plan"
sleep 1

tui_submit "Create a Python script that implements a web scraper"
tui_wait "plan|Plan|scraper|python|Python|step" 60 && test_pass "plan_mode" || test_fail "plan_mode"

tui_stop
test_summary
