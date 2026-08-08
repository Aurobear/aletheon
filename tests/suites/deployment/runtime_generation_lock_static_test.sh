#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
entrypoint="$ROOT/scripts/aletheon.sh"
suite="$ROOT/tests/coding/harness/suite.py"

grep -Fq 'ALETHEON_RUNTIME_LOCK_FILE' "$entrypoint"
grep -Fq 'exec flock -x -o "$runtime_lock" env' "$entrypoint"
grep -Fq 'ALETHEON_RUNTIME_LOCK_GUARD=1' "$entrypoint"
! grep -Fq 'exec {runtime_lock_fd}>"$runtime_lock"' "$entrypoint"
grep -Fq 'ALETHEON_RUNTIME_LOCK_FILE' "$suite"
grep -Fq 'fcntl.LOCK_SH' "$suite"

# `flock --close` must keep the supervisor lock while withholding its file
# descriptor from persistent deployment children.
lock=$(mktemp)
trap 'rm -f "$lock"' EXIT
LOCK_PATH=$lock flock -x -o "$lock" bash -c '
  ! find "/proc/$$/fd" -lname "'"$lock"'" -print -quit | grep -q .
  ! flock -n "$LOCK_PATH" true
'

echo 'runtime generation lock static test: pass'
