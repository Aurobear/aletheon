#!/usr/bin/env bash
# test_tui_integration.sh — TUI integration test orchestrator
# Usage: ./scripts/test_tui_integration.sh [scenario_name ...]
# If no args, runs all scenarios.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ALETHEON_BIN="$PROJECT_ROOT/target/release"
DAEMON_BIN="$ALETHEON_BIN/aletheond"
CLI_BIN="$ALETHEON_BIN/aletheon"
SCENARIOS_DIR="$PROJECT_ROOT/tests/tui_scenarios"
SOCKET="/tmp/aletheon-tui-test.sock"
TIMEOUT=120
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUTPUT_DIR="/tmp/aletheon-tui-test-$TIMESTAMP"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[TUI-TEST]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; }

cleanup() {
    log "Cleaning up..."
    if [[ -f "$OUTPUT_DIR/daemon.pid" ]]; then
        kill "$(cat "$OUTPUT_DIR/daemon.pid")" 2>/dev/null || true
    fi
    pkill -f "aletheond.*$SOCKET" 2>/dev/null || true
    rm -f "$SOCKET"
}
trap cleanup EXIT

# Check binaries
if [[ ! -x "$DAEMON_BIN" ]] || [[ ! -x "$CLI_BIN" ]]; then
    echo -e "${RED}Error: Binaries not found. Run 'cargo build --release' first.${NC}"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

# Start daemon
log "Starting daemon..."
rm -f "$SOCKET"
"$DAEMON_BIN" -s "$SOCKET" > "$OUTPUT_DIR/daemon.log" 2>&1 &
echo $! > "$OUTPUT_DIR/daemon.pid"
log "Daemon PID: $(cat "$OUTPUT_DIR/daemon.pid")"

# Wait for socket
WAIT=0
while [[ ! -S "$SOCKET" ]] && [[ $WAIT -lt 30 ]]; do
    sleep 1; WAIT=$((WAIT+1))
done
if [[ ! -S "$SOCKET" ]]; then
    fail "Daemon socket not ready after 30s"
    exit 1
fi
sleep 2
log "Daemon ready"

# Determine scenarios
SCENARIOS=("$@")
if [[ ${#SCENARIOS[@]} -eq 0 ]]; then
    for f in "$SCENARIOS_DIR"/*.txt; do
        SCENARIOS+=("$(basename "$f" .txt)")
    done
fi

log "Running ${#SCENARIOS[@]} scenarios..."
echo ""

PASS_COUNT=0
FAIL_COUNT=0

for scenario in "${SCENARIOS[@]}"; do
    INPUT_FILE="$SCENARIOS_DIR/${scenario}.txt"
    if [[ ! -f "$INPUT_FILE" ]]; then
        fail "Scenario file not found: $INPUT_FILE"
        FAIL_COUNT=$((FAIL_COUNT+1))
        continue
    fi

    log "Running scenario: $scenario"
    FRAMES_FILE="$OUTPUT_DIR/${scenario}_frames.jsonl"
    EVENTS_FILE="$OUTPUT_DIR/${scenario}_events.jsonl"

    # Run TUI with test flags
    timeout "$TIMEOUT" "$CLI_BIN" \
        -s "$SOCKET" \
        --test-input "$INPUT_FILE" \
        --record-frames "$FRAMES_FILE" \
        --record-events "$EVENTS_FILE" \
        --auto-submit \
        --test-timeout "$TIMEOUT" \
        > "$OUTPUT_DIR/${scenario}_tui.log" 2>&1 || true

    # Basic verification
    if [[ -f "$FRAMES_FILE" ]]; then
        FRAME_COUNT=$(wc -l < "$FRAMES_FILE")
        log "  Frames recorded: $FRAME_COUNT"
    else
        log "  No frames recorded"
    fi

    if [[ -f "$EVENTS_FILE" ]]; then
        EVENT_COUNT=$(wc -l < "$EVENTS_FILE")
        log "  Events recorded: $EVENT_COUNT"
    else
        log "  No events recorded"
    fi

    # Check for panic in daemon log
    if grep -qi "panic\|thread.*panicked" "$OUTPUT_DIR/daemon.log" 2>/dev/null; then
        fail "  [$scenario] Daemon panic detected"
        FAIL_COUNT=$((FAIL_COUNT+1))
    else
        pass "  [$scenario] No panic"
        PASS_COUNT=$((PASS_COUNT+1))
    fi
done

# Summary
echo ""
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "${CYAN}  TUI Integration Test Summary${NC}"
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "  ${GREEN}PASS: $PASS_COUNT${NC}"
echo -e "  ${RED}FAIL: $FAIL_COUNT${NC}"
echo -e "  Output: $OUTPUT_DIR"
echo ""

if [[ $FAIL_COUNT -gt 0 ]]; then
    exit 1
fi
