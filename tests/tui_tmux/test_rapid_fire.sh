#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"

test_begin "rapid_fire — 5 messages sequentially"
tui_start

# Send 5 messages one by one (each waits for previous to finish)
for i in 1 2 3 4 5; do
    tui_submit "test message $i"
    # Wait for response to appear (at least some text from the model)
    tui_wait "test message $i" 15
    # Wait a bit for the turn to complete
    sleep 3
done

# Final check: screen should show the last messages
tui_assert "test message 5"
test_pass "rapid_fire"

tui_stop
test_summary
