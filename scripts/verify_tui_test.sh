#!/usr/bin/env bash
# verify_tui_test.sh — Verify TUI test artifacts
# Usage: ./scripts/verify_tui_test.sh <output_dir>

set -euo pipefail

OUTPUT_DIR="${1:?Usage: $0 <output_dir>}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS_COUNT=0
FAIL_COUNT=0

pass() { echo -e "${GREEN}[PASS]${NC} $*"; PASS_COUNT=$((PASS_COUNT+1)); }
fail() { echo -e "${RED}[FAIL]${NC} $*"; FAIL_COUNT=$((FAIL_COUNT+1)); }

echo -e "${CYAN}Verifying TUI test artifacts in: $OUTPUT_DIR${NC}"
echo ""

# Check daemon log
DAEMON_LOG="$OUTPUT_DIR/daemon.log"
if [[ -f "$DAEMON_LOG" ]]; then
    if grep -qi "panic\|thread.*panicked" "$DAEMON_LOG"; then
        fail "Daemon panic detected"
    else
        pass "No daemon panic"
    fi

    ERROR_COUNT=$(grep -ci "error" "$DAEMON_LOG" 2>/dev/null || echo 0)
    if [[ $ERROR_COUNT -gt 0 ]]; then
        echo -e "${YELLOW}[WARN]${NC} Daemon log has $ERROR_COUNT error lines"
    fi
else
    fail "Daemon log not found"
fi

# Verify each scenario's artifacts
for frames_file in "$OUTPUT_DIR"/*_frames.jsonl; do
    [[ -f "$frames_file" ]] || continue
    scenario=$(basename "$frames_file" _frames.jsonl)
    events_file="$OUTPUT_DIR/${scenario}_events.jsonl"

    echo -e "\n${CYAN}Scenario: $scenario${NC}"

    # Check frames exist and are valid JSON
    FRAME_COUNT=$(wc -l < "$frames_file" 2>/dev/null || echo 0)
    if [[ $FRAME_COUNT -gt 0 ]]; then
        pass "Frames recorded: $FRAME_COUNT"
        # Validate JSON
        if jq empty "$frames_file" 2>/dev/null; then
            pass "Frames are valid JSONL"
        else
            fail "Frames contain invalid JSON"
        fi
    else
        fail "No frames recorded"
    fi

    # Check events exist and are valid JSON
    if [[ -f "$events_file" ]]; then
        EVENT_COUNT=$(wc -l < "$events_file" 2>/dev/null || echo 0)
        if [[ $EVENT_COUNT -gt 0 ]]; then
            pass "Events recorded: $EVENT_COUNT"
            # Check for turn_start
            if grep -q '"type":"turn_start"' "$events_file"; then
                pass "Has turn_start event"
            else
                fail "Missing turn_start event"
            fi
            # Check for turn_done
            if grep -q '"type":"turn_done"' "$events_file"; then
                pass "Has turn_done event"
            else
                fail "Missing turn_done event"
            fi
        else
            fail "No events recorded"
        fi
    else
        fail "Events file not found"
    fi

    # Check TUI log for errors
    TUI_LOG="$OUTPUT_DIR/${scenario}_tui.log"
    if [[ -f "$TUI_LOG" ]]; then
        if grep -qi "panic\|thread.*panicked" "$TUI_LOG"; then
            fail "TUI panic detected"
        else
            pass "No TUI panic"
        fi
    fi
done

# Summary
echo ""
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "${CYAN}  Verification Summary${NC}"
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "  ${GREEN}PASS: $PASS_COUNT${NC}"
echo -e "  ${RED}FAIL: $FAIL_COUNT${NC}"

if [[ $FAIL_COUNT -gt 0 ]]; then
    exit 1
fi
