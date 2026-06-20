#!/usr/bin/env bash
# run_all.sh — run all tmux TUI tests and report summary
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

TOTAL=0
PASSED=0
FAILED=0
FAILED_TESTS=()

for test_script in "$SCRIPT_DIR"/test_*.sh; do
    [[ -f "$test_script" ]] || continue
    name="$(basename "$test_script" .sh)"
    TOTAL=$((TOTAL + 1))

    echo ""
    echo -e "${CYAN}━━━ Running: $name ━━━${NC}"
    if bash "$test_script"; then
        PASSED=$((PASSED + 1))
    else
        FAILED=$((FAILED + 1))
        FAILED_TESTS+=("$name")
    fi
done

echo ""
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "${CYAN}  All Tests Summary${NC}"
echo -e "${CYAN}═══════════════════════════════════════════${NC}"
echo -e "  Total: $TOTAL"
echo -e "  ${GREEN}Pass:  $PASSED${NC}"
echo -e "  ${RED}Fail:  $FAILED${NC}"

if [[ ${#FAILED_TESTS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${RED}  Failed tests:${NC}"
    for t in "${FAILED_TESTS[@]}"; do
        echo -e "    - $t"
    done
    exit 1
fi

echo ""
echo -e "${GREEN}  All tests passed!${NC}"
exit 0
