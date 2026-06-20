#!/usr/bin/env bash
# test_tui_tmux.sh — tmux-based TUI integration test runner
# Usage: ./scripts/test_tui_tmux.sh [scenario_name ...]
# If no args, runs all scenarios.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TESTS_DIR="$PROJECT_ROOT/tests/tui_tmux"
SCENARIOS_DIR="$PROJECT_ROOT/tests/tui_scenarios"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[TMUX-RUNNER]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; }

# Determine scenarios
SCENARIOS=("$@")
if [[ ${#SCENARIOS[@]} -eq 0 ]]; then
    for f in "$TESTS_DIR"/test_*.sh; do
        name=$(basename "$f" .sh | sed 's/^test_//')
        SCENARIOS+=("$name")
    done
fi

log "Running ${#SCENARIOS[@]} scenarios..."
echo ""

PASS_COUNT=0
FAIL_COUNT=0

for scenario in "${SCENARIOS[@]}"; do
    TEST_SCRIPT="$TESTS_DIR/test_${scenario}.sh"
    if [[ ! -f "$TEST_SCRIPT" ]]; then
        fail "Test not found: $TEST_SCRIPT"
        FAIL_COUNT=$((FAIL_COUNT+1))
        continue
    fi

    chmod +x "$TEST_SCRIPT"
    if bash "$TEST_SCRIPT" 2>&1; then
        PASS_COUNT=$((PASS_COUNT+1))
    else
        FAIL_COUNT=$((FAIL_COUNT+1))
    fi
    echo ""
done

echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "${CYAN}  Final Summary${NC}"
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "  ${GREEN}Pass: $PASS_COUNT${NC}"
echo -e "  ${RED}Fail: $FAIL_COUNT${NC}"
echo ""

if [[ $FAIL_COUNT -gt 0 ]]; then
    exit 1
fi
