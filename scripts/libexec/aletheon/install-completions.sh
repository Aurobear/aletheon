#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
mode=${1:-}
destination=${2:-}

install_assets() {
  local bash_dir=$1 zsh_dir=$2
  install -d -m 0755 "$bash_dir" "$zsh_dir"
  install -m 0644 "$repo_root/scripts/completions/aletheon.bash" "$bash_dir/aletheon"
  install -m 0644 "$repo_root/scripts/completions/aletheon-ops.bash" "$bash_dir/aletheon.sh"
  install -m 0644 "$repo_root/scripts/completions/aletheon.zsh" "$zsh_dir/_aletheon"
  install -m 0644 "$repo_root/scripts/completions/aletheon-ops.zsh" "$zsh_dir/_aletheon.sh"
}

case "$mode" in
  --system)
    root=${destination:-/}
    install_assets \
      "$root/usr/share/bash-completion/completions" \
      "$root/usr/share/zsh/site-functions"
    ;;
  --user)
    data_home=${destination:-${XDG_DATA_HOME:-$HOME/.local/share}}
    install_assets \
      "$data_home/bash-completion/completions" \
      "$data_home/zsh/site-functions"
    ;;
  *)
    echo "usage: install-completions.sh {--system [root]|--user [data-home]}" >&2
    exit 2
    ;;
esac

echo "Aletheon Bash and Zsh completions installed; open a new shell to activate them"
