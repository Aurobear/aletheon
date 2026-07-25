#!/usr/bin/env bash
# lib.sh — tmux-based TUI test library
# Source this file in test scripts: source "$(dirname "$0")/lib.sh"

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI_BIN="${ALETHEON_TEST_BIN:-$HOME/.cache/aletheon-cargo/target/release/aletheon}"
[[ -x "$CLI_BIN" ]] || CLI_BIN="$PROJECT_ROOT/target/release/aletheon"
SOCKET_DIR="${XDG_RUNTIME_DIR:-/tmp}/aletheon-tmux-test-$$"
SOCKET="$SOCKET_DIR/aletheon.sock"
SESSION="aletheon-test-$$"
TMUX_WIDTH=120
TMUX_HEIGHT=40
DAEMON_PID=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[TMUX-TEST]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }

# ── Setup ───────────────────────────────────────────────────────

_cleanup() {
    log "Cleaning up..."
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET"
    rmdir "$SOCKET_DIR" 2>/dev/null || true
}

tui_start() {
    trap _cleanup EXIT

    # Check prerequisites
    if ! command -v tmux &>/dev/null; then
        fail "tmux not found. Install it first."
        exit 1
    fi
    if [[ ! -x "$CLI_BIN" ]]; then
        fail "Binary not found. Run: bash scripts/cargo-agent.sh build --release -p aletheon"
        exit 1
    fi

    # Kill stale sessions
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    rm -rf "$SOCKET_DIR"
    mkdir -m 700 -p "$SOCKET_DIR"

    # Start daemon
    log "Starting daemon..."
    "$CLI_BIN" daemon --socket "$SOCKET" &>/dev/null &
    DAEMON_PID=$!

    # Wait for socket
    local wait=0
    while [[ ! -S "$SOCKET" ]] && [[ $wait -lt 30 ]]; do
        sleep 0.5; wait=$((wait+1))
    done
    if [[ ! -S "$SOCKET" ]]; then
        fail "Daemon socket not ready after 15s"
        exit 1
    fi
    sleep 1
    log "Daemon ready (PID=$DAEMON_PID)"

    # Create tmux session
    tmux new-session -d -s "$SESSION" -x "$TMUX_WIDTH" -y "$TMUX_HEIGHT"
    sleep 0.5

    # Launch TUI inside tmux
    tmux send-keys -t "$SESSION" "$CLI_BIN -s $SOCKET" Enter
    sleep 2

    log "TUI launched in tmux session: $SESSION"
}

tui_stop() {
    tmux send-keys -t "$SESSION" C-c 2>/dev/null || true
    sleep 0.5
    tmux send-keys -t "$SESSION" C-c 2>/dev/null || true
    sleep 0.5
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET"
    rmdir "$SOCKET_DIR" 2>/dev/null || true
    DAEMON_PID=""
}

# ── Input ───────────────────────────────────────────────────────

tui_send() {
    tmux send-keys -t "$SESSION" "$1"
}

tui_key() {
    tmux send-keys -t "$SESSION" "$1"
}

tui_submit() {
    tmux send-keys -t "$SESSION" "$1" Enter
}

# ── Verification ────────────────────────────────────────────────

tui_capture() {
    tmux capture-pane -t "$SESSION" -p
}

tui_wait() {
    local pattern="$1"
    local timeout="${2:-60}"
    local elapsed=0

    while [[ $elapsed -lt $timeout ]]; do
        if tmux capture-pane -t "$SESSION" -p | grep -qE "$pattern"; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed+1))
    done

    fail "Timeout waiting for: '$pattern' (${timeout}s)"
    echo "--- Screen at timeout ---"
    tui_capture
    echo "--- End screen ---"
    return 1
}

tui_assert() {
    local pattern="$1"
    if ! tmux capture-pane -t "$SESSION" -p | grep -qE "$pattern"; then
        fail "Assertion failed: screen does not contain '$pattern'"
        echo "--- Current screen ---"
        tui_capture
        echo "--- End screen ---"
        return 1
    fi
}

tui_refute() {
    local pattern="$1"
    if tmux capture-pane -t "$SESSION" -p | grep -qE "$pattern"; then
        fail "Refutation failed: screen DOES contain '$pattern'"
        echo "--- Current screen ---"
        tui_capture
        echo "--- End screen ---"
        return 1
    fi
}

tui_wait_spinner() {
    # Wait until the spinner disappears (turn complete)
    local timeout="${1:-60}"
    local elapsed=0
    while [[ $elapsed -lt $timeout ]]; do
        if ! tmux capture-pane -t "$SESSION" -p | grep -q '⠋\|⠙\|⠹\|⠸\|⠼\|⠴\|⠦\|⠧\|⠇\|⠏'; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed+1))
    done
    fail "Spinner still active after ${timeout}s"
    return 1
}

# ── Reporting ───────────────────────────────────────────────────

TESTS_RUN=0
TESTS_PASS=0
TESTS_FAIL=0

test_begin() {
    TESTS_RUN=$((TESTS_RUN+1))
    log "[$TESTS_RUN] $1"
}

test_pass() {
    TESTS_PASS=$((TESTS_PASS+1))
    pass "$1"
}

test_fail() {
    TESTS_FAIL=$((TESTS_FAIL+1))
    fail "$1"
}

test_summary() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════${NC}"
    echo -e "${CYAN}  TUI tmux Test Summary${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════${NC}"
    echo -e "  Run:  $TESTS_RUN"
    echo -e "  ${GREEN}Pass: $TESTS_PASS${NC}"
    echo -e "  ${RED}Fail: $TESTS_FAIL${NC}"
    echo ""

    if [[ $TESTS_FAIL -gt 0 ]]; then
        exit 1
    fi
}
