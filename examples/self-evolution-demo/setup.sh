#!/usr/bin/env bash
# setup.sh -- Prepare environment for the self-evolution demo.
# Creates sample log files and a clean output directory.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DATA_DIR="$SCRIPT_DIR/sample-logs"
OUTPUT_DIR="$SCRIPT_DIR/output"

echo "[setup] Creating sample log files in $DATA_DIR ..."
mkdir -p "$DATA_DIR" "$OUTPUT_DIR"

# Generate synthetic log entries with varying severity
cat > "$DATA_DIR/auth.log" <<'EOF'
2026-06-14 08:00:01 [INFO] User alice logged in from 10.0.0.5
2026-06-14 08:00:03 [WARN] Failed login attempt for bob from 10.0.0.99
2026-06-14 08:00:05 [INFO] User alice logged out
2026-06-14 08:01:00 [ERROR] Authentication service timeout
2026-06-14 08:01:02 [INFO] Service recovered
EOF

cat > "$DATA_DIR/app.log" <<'EOF'
2026-06-14 08:00:02 [INFO] Request GET /health -> 200 (2ms)
2026-06-14 08:00:04 [WARN] Slow query detected: 1200ms
2026-06-14 08:00:10 [ERROR] Database connection pool exhausted
2026-06-14 08:00:12 [INFO] Pool replenished
2026-06-14 08:00:20 [INFO] Request GET /status -> 200 (1ms)
EOF

cat > "$DATA_DIR/system.log" <<'EOF'
2026-06-14 08:00:00 [INFO] System boot complete
2026-06-14 08:00:06 [WARN] CPU usage at 85%
2026-06-14 08:00:08 [ERROR] Disk /var is 95% full
2026-06-14 08:00:15 [WARN] Memory usage at 78%
2026-06-14 08:00:30 [INFO] Disk cleanup completed
EOF

echo "[setup] Done. Sample data ready."
