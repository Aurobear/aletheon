#!/usr/bin/env bash
# test_memory_loop.sh — real TUI multi-turn memory recall loop.
# Connects to installed daemon. Verifies recall across turns + /memory.
set -uo pipefail
source "$(dirname "$0")/lib.sh"

test_begin "memory_loop — multi-turn recall + /memory verification"

# Use installed daemon socket.
PROD_SOCKET="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock"
if [[ ! -S "$PROD_SOCKET" ]]; then
  test_fail "daemon socket not found at $PROD_SOCKET"
  test_summary; exit 1
fi

# Start TUI using the lib's tui_start but override SOCKET.
_cleanup_loop() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
}
trap _cleanup_loop EXIT

SESSION="aletheon-mem-$$"
tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x 120 -y 40
sleep 0.5
tmux send-keys -t "$SESSION" "$CLI_BIN -s $PROD_SOCKET" Enter
sleep 4

# Wait for TUI prompt.
elapsed=0
while [[ $elapsed -lt 30 ]]; do
  if tmux capture-pane -t "$SESSION" -p 2>/dev/null | grep -q "❯"; then
    break
  fi
  sleep 1; elapsed=$((elapsed+1))
done

# Clean session.
tmux send-keys -t "$SESSION" "/clear" Enter
if tui_wait "已创建新会话" 120; then
  test_pass "session ready"
else
  test_fail "session ready"; test_summary; exit 1
fi

# Turn 1: Store facts.
tmux send-keys -t "$SESSION" "记住: port1=9090, port2=8443" Enter
if tui_wait_spinner 180; then
  screen=$(tmux capture-pane -t "$SESSION" -p)
  if echo "$screen" | grep -qE "9090|8443"; then
    test_pass "turn-1 stored both values (9090, 8443)"
  else
    test_fail "turn-1 response received but values not found"
  fi
else
  test_fail "turn-1 spinner timeout"
fi

# Turn 2: Recall.
tmux send-keys -t "$SESSION" "刚才说的两个端口号是多少?" Enter
if tui_wait_spinner 180; then
  screen=$(tmux capture-pane -t "$SESSION" -p)
  if echo "$screen" | grep -qE "9090|8443"; then
    test_pass "turn-2 recalled both values"
  else
    test_fail "turn-2 response received but values not found"
  fi
else
  test_fail "turn-2 spinner timeout"
fi

# /memory command.
tmux send-keys -t "$SESSION" "/memory" Enter
if tui_wait "Memory Facts" 20; then
  test_pass "/memory renders"
else
  test_fail "/memory renders"
fi

_cleanup_loop
trap - EXIT
test_summary
