#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
entrypoint="$ROOT/scripts/aletheon.sh"

# A deploy launched through sudo must re-enter as the invoking account before
# common.sh derives HOME-based paths or any `systemctl --user` command runs.
reentry_line=$(grep -n 'ALETHEON_DEPLOY_AS_USER=1' "$entrypoint" | cut -d: -f1)
common_line=$(grep -n 'source "$SCRIPT_DIR/lib/aletheon/common.sh"' "$entrypoint" | cut -d: -f1)
test "$reentry_line" -lt "$common_line"

grep -Fq 'XDG_RUNTIME_DIR=/run/user/$deploy_uid' "$entrypoint"
grep -Fq 'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$deploy_uid/bus' "$entrypoint"
grep -Fq 'HOME=$deploy_home' "$entrypoint"
grep -Fq 'exec sudo -u "$deploy_user" -H env' "$entrypoint"

echo 'sudo deploy user context static test: pass'
