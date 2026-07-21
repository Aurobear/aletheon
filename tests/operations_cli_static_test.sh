#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
entry="$root/scripts/aletheon.sh"

bash -n "$entry" "$root"/scripts/lib/aletheon/*.sh
[[ -x "$entry" ]]

for command in build install deploy configure status health restart logs verify closure help; do
  bash "$entry" help | grep -q "$command"
done

grep -q 'scripts/cargo-agent.sh.*build -p aletheon --release' \
  "$root/scripts/lib/aletheon/build.sh"
grep -q 'sudo env ALETHEON_BINARY=' "$root/scripts/lib/aletheon/install.sh"
! grep -R -E '(API_KEY|TOKEN)=.+' "$root/scripts/aletheon.sh" "$root/scripts/lib/aletheon"

echo 'operations CLI static tests passed'
