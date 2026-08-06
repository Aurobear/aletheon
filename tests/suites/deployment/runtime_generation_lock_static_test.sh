#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
entrypoint="$ROOT/scripts/aletheon.sh"
suite="$ROOT/tests/coding/harness/suite.py"

grep -Fq 'ALETHEON_RUNTIME_LOCK_FILE' "$entrypoint"
grep -Fq 'flock -x "$runtime_lock_fd"' "$entrypoint"
grep -Fq 'ALETHEON_RUNTIME_LOCK_FILE' "$suite"
grep -Fq 'fcntl.LOCK_SH' "$suite"

echo 'runtime generation lock static test: pass'
