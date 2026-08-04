#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)

source "$root/scripts/completions/aletheon.bash"
source "$root/scripts/completions/aletheon-ops.bash"
complete -p aletheon | grep -q '_aletheon_cli_completion'
complete -p aletheon.sh | grep -q '_aletheon_operations_completion'

COMP_WORDS=(aletheon mem); COMP_CWORD=1
_aletheon_cli_completion
printf '%s\n' "${COMPREPLY[@]}" | grep -qx memory

COMP_WORDS=(aletheon ''); COMP_CWORD=1
_aletheon_cli_completion
for retired in reflect reflect_now evolution genome hooks task evaluation approve plan computer; do
  if printf '%s\n' "${COMPREPLY[@]}" | grep -qx "$retired"; then
    echo "retired governance command remains in CLI completion: $retired" >&2
    exit 1
  fi
done

COMP_WORDS=(aletheon memory ''); COMP_CWORD=2
_aletheon_cli_completion
for expected in observe recall receipt workspace; do
  printf '%s\n' "${COMPREPLY[@]}" | grep -qx "$expected"
done

COMP_WORDS=(aletheon.sh dep); COMP_CWORD=1
_aletheon_operations_completion
printf '%s\n' "${COMPREPLY[@]}" | grep -qx deploy

grep -q '^compdef _aletheon_cli_completion aletheon$' "$root/scripts/completions/aletheon.zsh"
grep -q '^compdef _aletheon_operations_completion aletheon.sh$' "$root/scripts/completions/aletheon-ops.zsh"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
bash "$root/scripts/libexec/aletheon/install-completions.sh" --system "$tmp"
for path in \
  usr/share/bash-completion/completions/aletheon \
  usr/share/bash-completion/completions/aletheon.sh \
  usr/share/zsh/site-functions/_aletheon \
  usr/share/zsh/site-functions/_aletheon.sh; do
  test -f "$tmp/$path"
  test "$(stat -c %a "$tmp/$path")" = 644
done

# Installation is idempotent.
bash "$root/scripts/libexec/aletheon/install-completions.sh" --system "$tmp"

echo "completion contracts passed"
