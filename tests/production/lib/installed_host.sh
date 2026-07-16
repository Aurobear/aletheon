#!/usr/bin/env bash
# Safety and evidence helpers for tests that mutate an installed Aletheon host.
set -euo pipefail

production_repo_root() { cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P; }

require_disposable_installed_host() {
  [[ ${ALETHEON_DISPOSABLE_HOST:-0} == 1 ]] || {
    echo "BLOCKED: set ALETHEON_DISPOSABLE_HOST=1 only inside a disposable systemd VM/container" >&2; return 78;
  }
  [[ ${EUID:-$(id -u)} -eq 0 ]] || {
    echo "BLOCKED: installed-host lane must be root inside the disposable guest" >&2; return 78;
  }
  local virtualization
  virtualization=$(systemd-detect-virt --container 2>/dev/null || systemd-detect-virt --vm 2>/dev/null || true)
  [[ -n "$virtualization" ]] || {
    echo "REFUSED: no VM/container evidence; development host will not be modified" >&2; return 78;
  }
  [[ -d /run/systemd/system ]] || {
    echo "BLOCKED: disposable guest is not booted with systemd" >&2; return 78;
  }
  for command in systemctl systemd-analyze journalctl sqlite3 jq sha256sum ss; do
    command -v "$command" >/dev/null || { echo "BLOCKED: disposable guest missing $command" >&2; return 78; }
  done
}

init_release_artifacts() {
  local root=${ALETHEON_RELEASE_ARTIFACTS:-/var/tmp/aletheon-release-acceptance}
  [[ "$root" == /var/tmp/* || "$root" == /tmp/* ]] || {
    echo "REFUSED: artifact root must be below /var/tmp or /tmp" >&2; return 78;
  }
  install -d -m 0700 "$root"
  printf '%s\n' "$root"
}

assert_installed_boundaries() {
  local artifacts=$1
  stat -c '%n %U:%G %a' /usr/bin/aletheon /etc/aletheon/config.toml \
    /var/lib/aletheon /run/aletheon >"$artifacts/modes.txt"
  [[ -S /run/aletheon/aletheon.sock ]] || { echo "installed daemon socket is not AF_UNIX" >&2; return 1; }
  ss -xl | grep -F '/run/aletheon/aletheon.sock' >"$artifacts/af-unix.txt"
  systemctl show aletheon.service -p ActiveState -p SubState -p MainPID -p ExecMainStatus \
    >"$artifacts/unit-state.txt"
  grep -qx 'ActiveState=active' "$artifacts/unit-state.txt"
}

capture_sqlite_integrity() {
  local root=$1 output=$2
  local count=0
  : >"$output"
  while IFS= read -r -d '' database; do
    count=$((count + 1))
    printf '%s\t' "${database#"$root"/}" >>"$output"
    sqlite3 "$database" 'PRAGMA integrity_check;' >>"$output"
  done < <(find "$root" -xdev -type f \( -name '*.db' -o -name '*.sqlite' \) -print0)
  ((count > 0)) || { echo "no SQLite databases found below $root" >&2; return 1; }
  ! grep -v $'\tok$' "$output" | grep -q .
}
