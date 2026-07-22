#!/usr/bin/env bash
# Explicit, offline SQLite quick-check for Aletheon state databases.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: scripts/aletheon-sqlite-check.sh DATABASE [DATABASE ...]

Runs SQLite PRAGMA quick_check against existing databases in read-only mode.
Stop the owning Aletheon service, or operate on a consistent backup, before
using this diagnostic on a live production database.
EOF
}

(( $# > 0 )) || { usage; exit 2; }
command -v sqlite3 >/dev/null || {
  echo "missing required command: sqlite3" >&2
  exit 127
}

for database in "$@"; do
  [[ -f "$database" ]] || {
    echo "not an existing regular database: $database" >&2
    exit 2
  }

  result=$(sqlite3 -readonly "$database" ".timeout 30000" "PRAGMA quick_check;") || {
    echo "SQLite quick_check could not run: $database" >&2
    exit 1
  }
  if [[ "$result" != "ok" ]]; then
    echo "SQLite quick_check failed: $database" >&2
    printf '%s\n' "$result" >&2
    exit 1
  fi
  echo "SQLite quick_check passed: $database"
done
