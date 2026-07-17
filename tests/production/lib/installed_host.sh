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
  for command in systemctl systemd-analyze journalctl sqlite3 jq sha256sum ss \
    getent loginctl runuser usermod python3; do
    command -v "$command" >/dev/null || { echo "BLOCKED: disposable guest missing $command" >&2; return 78; }
  done
}

installed_test_users() {
  [[ -n ${ALETHEON_TEST_USER_A:-} && -n ${ALETHEON_TEST_USER_B:-} ]] || {
    echo "BLOCKED: set two explicit ALETHEON_TEST_USER_A/B disposable-guest accounts" >&2
    return 78
  }
  [[ "$ALETHEON_TEST_USER_A" != "$ALETHEON_TEST_USER_B" ]] || {
    echo "BLOCKED: installed-host users must be distinct" >&2; return 78;
  }
  printf '%s\n' "$ALETHEON_TEST_USER_A" "$ALETHEON_TEST_USER_B"
}

installed_user_uid() { id -u "$1"; }

installed_user_home() {
  local user=$1 entry home
  entry=$(getent passwd "$user") || {
    echo "BLOCKED: installed-host user does not exist: $user" >&2; return 78;
  }
  IFS=: read -r _ _ uid _ _ home _ <<<"$entry"
  [[ "$uid" =~ ^[0-9]+$ && "$uid" -ne 0 && "$user" != aletheon ]] || {
    echo "BLOCKED: installed-host test principal must be unprivileged: $user" >&2; return 78;
  }
  [[ "$home" == /* && -d "$home" && ! -L "$home" ]] || {
    echo "BLOCKED: installed-host user has an unsafe home: $user" >&2; return 78;
  }
  printf '%s\n' "$home"
}

installed_user_socket() {
  printf '/run/user/%s/aletheon/aletheon.sock\n' "$(installed_user_uid "$1")"
}

installed_user_state_root_from_home() {
  local home=$1
  [[ "$home" == /* && "$home" != / ]] || {
    echo "refusing unsafe user-state home" >&2; return 78;
  }
  printf '%s/.local/state/aletheon\n' "${home%/}"
}

installed_user_state_root() {
  installed_user_state_root_from_home "$(installed_user_home "$1")"
}

run_as_installed_user() {
  local user=$1 uid home
  shift
  uid=$(installed_user_uid "$user")
  home=$(installed_user_home "$user")
  runuser -u "$user" -- env \
    "HOME=$home" \
    "XDG_RUNTIME_DIR=/run/user/$uid" \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus" \
    "$@"
}

prepare_installed_test_users() {
  local user uid
  systemctl --global enable aletheon.socket
  while IFS= read -r user; do
    installed_user_home "$user" >/dev/null
    uid=$(installed_user_uid "$user")
    usermod -a -G aletheon "$user"
    id -nG "$user" | tr ' ' '\n' | grep -qx aletheon || {
      echo "failed to authorize installed-host user for the core: $user" >&2; return 1;
    }
    systemctl stop "user@$uid.service" >/dev/null 2>&1 || true
    loginctl enable-linger "$user"
    systemctl start "user@$uid.service"
    [[ -d "/run/user/$uid" ]] || {
      echo "BLOCKED: user runtime directory was not created for $user" >&2; return 78;
    }
    run_as_installed_user "$user" systemctl --user daemon-reload
    run_as_installed_user "$user" systemctl --user show-environment >/dev/null
  done < <(installed_test_users)
}

start_installed_runtime() {
  local user
  systemctl start aletheon-core.service
  while IFS= read -r user; do
    run_as_installed_user "$user" systemctl --user start aletheon.socket
  done < <(installed_test_users)
}

stop_installed_runtime() {
  local user
  while IFS= read -r user; do
    run_as_installed_user "$user" systemctl --user stop aletheon.socket aletheon.service
    if run_as_installed_user "$user" systemctl --user is-active --quiet aletheon.service; then
      echo "user daemon did not stop cleanly: $user" >&2
      return 1
    fi
  done < <(installed_test_users)
  systemctl stop aletheon-core.service
  if systemctl is-active --quiet aletheon-core.service; then
    echo "system inference core did not stop cleanly" >&2
    return 1
  fi
}

restart_installed_runtime() {
  local user
  systemctl restart aletheon-core.service
  while IFS= read -r user; do
    run_as_installed_user "$user" systemctl --user stop aletheon.service aletheon.socket
    run_as_installed_user "$user" systemctl --user start aletheon.socket
  done < <(installed_test_users)
}

assert_installed_readiness() {
  local user socket
  systemctl is-active --quiet aletheon-core.service
  [[ -S /run/aletheon/core.sock ]] || {
    echo "installed core socket is not ready" >&2
    return 1
  }
  [[ $(stat -c %a /run/aletheon/core.sock) == 660 ]] || {
    echo "installed core socket has unsafe mode" >&2
    return 1
  }
  ss -xl | grep -Fq '/run/aletheon/core.sock'
  while IFS= read -r user; do
    socket=$(installed_user_socket "$user")
    run_as_installed_user "$user" /usr/libexec/aletheon/verify-systemd.sh \
      --readiness --socket "$socket" --timeout 30
  done < <(installed_test_users)
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
  local user uid gid socket
  stat -c '%n %U:%G %a' /usr/bin/aletheon /etc/aletheon/config.toml \
    /var/lib/aletheon /run/aletheon >"$artifacts/modes.txt"
  [[ -S /run/aletheon/core.sock ]] || { echo "installed core socket is not AF_UNIX" >&2; return 1; }
  [[ $(stat -c '%U:%G:%a' /run/aletheon/core.sock) == aletheon:aletheon:660 ]] || {
    echo "installed core socket has unsafe ownership or mode" >&2; return 1;
  }
  ss -xl | grep -F '/run/aletheon/core.sock' >"$artifacts/af-unix.txt"
  systemctl show aletheon-core.service -p ActiveState -p SubState -p MainPID -p ExecMainStatus \
    >"$artifacts/unit-state.txt"
  grep -qx 'ActiveState=active' "$artifacts/unit-state.txt"
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    gid=$(id -g "$user")
    socket=$(installed_user_socket "$user")
    [[ -S "$socket" ]] || { echo "installed user socket is not AF_UNIX: $user" >&2; return 1; }
    [[ $(stat -c %a "$socket") == 600 ]] || { echo "unsafe socket mode for $user" >&2; return 1; }
    [[ $(stat -c %u "$socket") == "$uid" && $(stat -c %g "$socket") == "$gid" ]] || {
      echo "unsafe socket ownership for $user" >&2; return 1;
    }
    stat -c '%n %U:%G %a' "$socket" >>"$artifacts/modes.txt"
    ss -xl | grep -F "$socket" >>"$artifacts/af-unix.txt"
    run_as_installed_user "$user" systemctl --user show \
      aletheon.socket aletheon.service -p Id -p ActiveState -p SubState -p MainPID \
      >"$artifacts/user-unit-state.$uid.txt"
    grep -qx 'Id=aletheon.socket' "$artifacts/user-unit-state.$uid.txt"
    grep -qx 'Id=aletheon.service' "$artifacts/user-unit-state.$uid.txt"
    [[ $(grep -cx 'ActiveState=active' "$artifacts/user-unit-state.$uid.txt") -eq 2 ]] || {
      echo "installed user socket/service is not active: $user" >&2; return 1;
    }
  done < <(installed_test_users)

  local user_a=$ALETHEON_TEST_USER_A socket_b
  socket_b=$(installed_user_socket "$ALETHEON_TEST_USER_B")
  if run_as_installed_user "$user_a" python3 - "$socket_b" <<'PY'
import socket, sys
client = socket.socket(socket.AF_UNIX)
client.connect(sys.argv[1])
PY
  then
    echo "cross-user private socket connection unexpectedly succeeded" >&2
    return 1
  fi
}

capture_installed_journal() {
  local output=$1 user uid
  journalctl -u aletheon-core.service --no-pager >"$output/core.txt"
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    journalctl --no-pager _UID="$uid" _SYSTEMD_USER_UNIT=aletheon.service \
      >"$output/user-$uid.txt"
  done < <(installed_test_users)
}

backup_installed_user_state() {
  local output=$1 user uid state target
  install -d -m 0700 "$output"
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    state=$(installed_user_state_root "$user")
    [[ -d "$state" && ! -L "$state" ]] || {
      echo "user state root is unavailable for $user" >&2; return 1;
    }
    target="$output/$uid"
    install -d -m 0700 "$target"
    cp -a -- "$state" "$target/state"
    find "$target/state" -type f -print0 | sort -z | xargs -0 -r sha256sum \
      >"$target/MANIFEST.sha256"
  done < <(installed_test_users)
}

archive_installed_user_state() {
  local output=$1 user uid state
  install -d -m 0700 "$output"
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    state=$(installed_user_state_root "$user")
    [[ -d "$state" && ! -L "$state" ]] || continue
    mv -- "$state" "$output/$uid"
  done < <(installed_test_users)
}

restore_installed_user_state() {
  local source=$1 user uid state parent
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    state=$(installed_user_state_root "$user")
    parent=${state%/aletheon}
    [[ ! -e "$state" && -d "$source/$uid/state" && ! -L "$source/$uid/state" ]] || {
      echo "refusing unsafe user-state restore for $user" >&2; return 1;
    }
    install -d -o "$uid" -g "$(id -g "$user")" -m 0700 "$parent"
    cp -a -- "$source/$uid/state" "$state"
  done < <(installed_test_users)
}

capture_installed_user_integrity() {
  local output=$1 user uid state
  : >"$output"
  while IFS= read -r user; do
    uid=$(installed_user_uid "$user")
    state=$(installed_user_state_root "$user")
    printf 'user=%s uid=%s\n' "$user" "$uid" >>"$output"
    capture_sqlite_integrity "$state" "$output.$uid"
    cat "$output.$uid" >>"$output"
  done < <(installed_test_users)
}

installed_host_user_state_static_test() {
  [[ $(installed_user_state_root_from_home /home/v02-user) == \
    /home/v02-user/.local/state/aletheon ]]
  local legacy='.local/'"share/aletheon"
  ! grep -F "$legacy" "${BASH_SOURCE[0]}"
  echo "installed-host user-state path verification: pass"
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

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  case "${1-}" in
    --static-test) installed_host_user_state_static_test ;;
    *) echo "usage: $0 --static-test" >&2; exit 64 ;;
  esac
fi
