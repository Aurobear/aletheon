#!/usr/bin/env bash
# run-demo.sh -- Execute the self-evolution demo (two-pass learning loop).
#
# This script simulates the Aletheon self-evolution pipeline:
#   Run 1: agent produces a raw report, reflects, and stores a learned rule.
#   Run 2: agent applies the learned rule and produces an improved report.
#
# In production the ReAct loop handles this automatically; here we walk
# through the same steps with shell + the aletheon-cli binary for clarity.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DATA_DIR="$SCRIPT_DIR/sample-logs"
OUTPUT_DIR="$SCRIPT_DIR/output"
CONFIG="$SCRIPT_DIR/config.toml"

# Ensure setup has been run
if [ ! -d "$DATA_DIR" ]; then
    echo "[demo] Sample data not found. Running setup.sh first ..."
    bash "$SCRIPT_DIR/setup.sh"
fi

echo ""
echo "=== Run 1: Initial execution ==="
echo "[INFO] Processing log files from $DATA_DIR ..."

# Pass 1: raw concatenation (baseline)
cat "$DATA_DIR"/*.log | sort > "$OUTPUT_DIR/run1-report.txt"
echo "[INFO] Output written to output/run1-report.txt"

# Reflection phase (simulated)
echo "[REFLECT] Noted: raw output is too verbose, should group by severity."
echo "[LEARN] Stored rule: group-by-severity"

echo ""
echo "=== Run 2: After learning ==="
echo "[INFO] Processing log files from $DATA_DIR ..."

# Pass 2: grouped by severity (learned behavior)
{
    echo "--- ERRORS ---"
    grep '\[ERROR\]' "$DATA_DIR"/*.log | sort
    echo ""
    echo "--- WARNINGS ---"
    grep '\[WARN\]' "$DATA_DIR"/*.log | sort
    echo ""
    echo "--- INFO ---"
    grep '\[INFO\]' "$DATA_DIR"/*.log | sort
} > "$OUTPUT_DIR/run2-report.txt"

echo "[INFO] Output written to output/run2-report.txt"
echo "[REFLECT] Output quality improved. Rule applied: group-by-severity"
echo "[LEARN] No new rules -- output meets quality bar."

echo ""
echo "=== Demo complete ==="
echo "Compare: diff output/run1-report.txt output/run2-report.txt"
