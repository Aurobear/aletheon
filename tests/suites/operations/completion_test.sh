#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
cd "$root"

complete_word() {
  local -a COMP_WORDS=("$@")
  local COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
  local -a COMPREPLY=()
  _aletheon_operations_completion
  printf '%s\n' "${COMPREPLY[@]}"
}

# shellcheck source=../../../scripts/completions/aletheon.bash
source scripts/completions/aletheon.bash

grep -qx test < <(complete_word scripts/aletheon.sh te)
grep -qx operations < <(complete_word scripts/aletheon.sh test op)
grep -qx -- --no-restart < <(complete_word scripts/aletheon.sh deploy --no-r)
grep -qx multi-user < <(complete_word scripts/aletheon.sh verify multi)
grep -qx zsh < <(complete_word scripts/aletheon.sh completion zs)

cmp -s scripts/completions/aletheon.bash <(scripts/aletheon.sh completion bash)
cmp -s scripts/completions/aletheon.zsh <(scripts/aletheon.sh completion zsh)

bash -n scripts/completions/aletheon.bash
if command -v zsh >/dev/null 2>&1; then
  zsh -n scripts/completions/aletheon.zsh
fi

grep -Fq '/usr/share/bash-completion/completions/aletheon.sh' setup.sh
grep -Fq '/usr/share/zsh/site-functions/_aletheon.sh' setup.sh

echo 'shell completion: pass'
