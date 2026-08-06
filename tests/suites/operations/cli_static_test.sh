#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
entry="$root/scripts/aletheon.sh"

bash -n "$entry" "$root"/scripts/lib/aletheon/*.sh
[[ -x "$entry" ]]
grep -Fq 'source "$SCRIPT_DIR/lib/aletheon/runtime_gate.sh"' "$entry"
grep -Fq 'cmd_installed_runtime_gate' "$root/scripts/lib/aletheon/verify.sh"
grep -Fq 'robot-r8)' \
  "$root/scripts/lib/aletheon/acceptance.sh"
grep -Fq 'run_internal robot_r8_evidence.py "$@"' \
  "$root/scripts/lib/aletheon/acceptance.sh"
grep -Fq 'cmd_installed_runtime_gate || return' \
  "$root/scripts/lib/aletheon/acceptance.sh"
grep -Fq 'remove_user_cli_shadow' "$root/scripts/lib/aletheon/install.sh"
grep -Fq 'PATH resolves aletheon to a stale binary' "$root/scripts/lib/aletheon/verify.sh"
grep -Fq 'ALETHEON_DEPLOY_SCOPE=system' "$root/scripts/aletheon.sh"
if grep -Fq 'sudo -n true' "$root/scripts/aletheon.sh"; then
  echo 'deploy scope must not be inferred from passwordless sudo availability' >&2
  exit 1
fi

for command in build install deploy configure status health restart logs verify closure \
  backup restore upgrade cleanup secrets database acceptance test help; do
  bash "$entry" help | grep -q "$command"
done

grep -q 'scripts/cargo-agent.sh.*build -p aletheon --release' \
  "$root/scripts/lib/aletheon/build.sh"
grep -q 'sudo env ALETHEON_BINARY=' "$root/scripts/lib/aletheon/install.sh"
! grep -R -E '(API_KEY|TOKEN)=.+' "$root/scripts/aletheon.sh" "$root/scripts/lib/aletheon"

echo 'operations CLI static tests passed'
