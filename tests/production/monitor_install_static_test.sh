#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
install_lib="$repo_root/scripts/lib/aletheon/install.sh"
setup="$repo_root/setup.sh"

grep -Fq 'python3 -m venv "$venv_dir"' "$install_lib"
grep -Fq '"$venv_dir/bin/python" -m pip install' "$install_lib"
grep -Fq 'exec "$venv_dir/bin/python" "$monitor_dir/run.py"' "$install_lib"
grep -Fq "from src.server import server" "$install_lib"
grep -Fq 'cmd_monitor_install' "$install_lib"

grep -Fq 'python3 -m venv "$venv"' "$setup"
grep -Fq 'exec "$venv/bin/python" "$MONITOR_DST/run.py"' "$setup"

if grep -Fq 'exec python3 "$MONITOR_DST/run.py"' "$setup"; then
  echo "setup monitor still uses ambient system Python" >&2
  exit 1
fi

echo "monitor isolated-install static verification: pass"
