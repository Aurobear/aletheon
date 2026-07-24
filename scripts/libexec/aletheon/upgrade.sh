#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: upgrade-aletheon.sh --binary FILE --sha256-file FILE [--config FILE]
       [--assets DIR] [--authorized-users FILE] [--user-backup-command FILE]
       [--legacy-single-daemon]

The default production topology upgrades aletheon-core.service plus the
aletheon.socket/aletheon.service user runtimes named by --authorized-users.
Legacy single-daemon operation must be selected explicitly.
EOF
  exit 64
}

test_mode=${ALETHEON_UNPRIVILEGED_TEST:-0}
[[ ${EUID:-$(id -u)} -eq 0 || "$test_mode" == 1 ]] || { echo "run as root" >&2; exit 1; }
binary=
sha_file=
config=/etc/aletheon/config.toml
assets=
topology=${ALETHEON_UPGRADE_TOPOLOGY:-multi-user}
authorized_users_file=${ALETHEON_AUTHORIZED_USERS_FILE:-}
user_backup_command=${ALETHEON_USER_BACKUP_COMMAND:-}
while (($#)); do
  case "$1" in
    --binary) binary=${2:-}; shift 2 ;;
    --sha256-file) sha_file=${2:-}; shift 2 ;;
    --config) config=${2:-}; shift 2 ;;
    --assets) assets=${2:-}; shift 2 ;;
    --authorized-users) authorized_users_file=${2:-}; shift 2 ;;
    --user-backup-command) user_backup_command=${2:-}; shift 2 ;;
    --legacy-single-daemon) topology=legacy; shift ;;
    *) usage ;;
  esac
done
[[ "$topology" == multi-user || "$topology" == legacy ]] || { echo "invalid upgrade topology" >&2; exit 64; }
[[ -n "$binary" && -n "$sha_file" && -x "$binary" && -f "$sha_file" && ! -L "$binary" && ! -L "$sha_file" ]] || usage
[[ -f "$config" && ! -L "$config" ]] || { echo "invalid production config" >&2; exit 1; }
if [[ -n "$assets" ]]; then
  [[ -d "$assets" && ! -L "$assets" && -f "$assets/MANIFEST.sha256" ]] || { echo "invalid asset bundle" >&2; exit 1; }
  (cd -- "$assets" && sha256sum -c MANIFEST.sha256)
fi

expected=$(awk 'NF>=1 {print $1; exit}' "$sha_file")
[[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || { echo "invalid binary checksum file" >&2; exit 1; }
actual=$(sha256sum -- "$binary" | cut -d' ' -f1)
[[ "$actual" == "${expected,,}" ]] || { echo "binary checksum mismatch" >&2; exit 1; }
version=$($binary version | head -n1)
[[ -n "$version" ]] || { echo "candidate binary returned no version" >&2; exit 1; }

prefix=${ALETHEON_INSTALL_PREFIX:-/usr}
release_root=${ALETHEON_RELEASE_ROOT:-/var/lib/aletheon/releases}
receipt_root=${ALETHEON_RECEIPT_ROOT:-/var/lib/aletheon/state/upgrades}
backup_command=${ALETHEON_BACKUP_COMMAND:-/usr/libexec/aletheon/backup-aletheon.sh}
preflight=${ALETHEON_PREFLIGHT_COMMAND:-/usr/libexec/aletheon/verify-systemd.sh}
healthcheck=${ALETHEON_HEALTHCHECK_COMMAND:-/usr/libexec/aletheon/aletheon-healthcheck.sh}
systemctl_command=${ALETHEON_SYSTEMCTL_COMMAND:-systemctl}
runuser_command=${ALETHEON_RUNUSER_COMMAND:-runuser}
core_unit=${ALETHEON_CORE_UNIT:-aletheon-core.service}
core_socket=${ALETHEON_CORE_SOCKET:-/run/aletheon/core.sock}
legacy_unit=${ALETHEON_LEGACY_UNIT:-aletheon.service}
legacy_socket=${ALETHEON_LEGACY_SOCKET:-/run/aletheon/aletheon.sock}
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
user_backup_root=${ALETHEON_USER_BACKUP_ROOT:-$receipt_root/user-backups/$timestamp}

if [[ "$test_mode" == 1 ]]; then
  case "$prefix" in /tmp/*) ;; *) echo "unprivileged test prefix must be below /tmp" >&2; exit 1 ;; esac
fi
if [[ "$test_mode" == 1 ]]; then
  install -d -m 0755 "$release_root"
  install -d -m 0750 "$receipt_root"
else
  install -d -o root -g root -m 0755 "$release_root"
  install -d -o root -g aletheon -m 0750 "$receipt_root"
fi

[[ -x "$backup_command" && ! -L "$backup_command" ]] || { echo "backup command is unavailable or unsafe" >&2; exit 1; }
[[ -x "$preflight" && ! -L "$preflight" ]] || { echo "preflight command is unavailable or unsafe" >&2; exit 1; }
[[ -x "$healthcheck" && ! -L "$healthcheck" ]] || { echo "healthcheck command is unavailable or unsafe" >&2; exit 1; }

declare -a authorized_users=()
declare -a authorized_uids=()
declare -a authorized_homes=()
if [[ "$topology" == multi-user ]]; then
  [[ -n "$authorized_users_file" && -f "$authorized_users_file" && ! -L "$authorized_users_file" ]] || {
    echo "multi-user upgrade requires a regular --authorized-users manifest" >&2; exit 1;
  }
  [[ -n "$user_backup_command" && -x "$user_backup_command" && ! -L "$user_backup_command" ]] || {
    echo "multi-user upgrade requires an executable --user-backup-command" >&2; exit 1;
  }
  if [[ "$test_mode" != 1 ]]; then
    [[ $(stat -c %u "$authorized_users_file") == 0 && $((8#$(stat -c %a "$authorized_users_file") & 8#022)) -eq 0 ]] || {
      echo "authorized-user manifest must be root-owned and not group/world writable" >&2; exit 1;
    }
  fi
  while IFS= read -r user; do
    [[ -n "$user" ]] || continue
    [[ "$user" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] || { echo "invalid authorized user: $user" >&2; exit 1; }
    [[ "$user" != root && "$user" != aletheon ]] || { echo "refusing privileged/service principal: $user" >&2; exit 1; }
    for prior in "${authorized_users[@]}"; do
      [[ "$prior" != "$user" ]] || { echo "duplicate authorized user: $user" >&2; exit 1; }
    done
    entry=$(getent passwd "$user") || { echo "authorized user does not exist: $user" >&2; exit 1; }
    IFS=: read -r _ _ uid _ _ home _ <<<"$entry"
    [[ "$uid" =~ ^[0-9]+$ && "$uid" -ne 0 && "$home" == /* && -d "$home" && ! -L "$home" ]] || {
      echo "authorized user has unsafe identity/home: $user" >&2; exit 1;
    }
    if [[ "$test_mode" != 1 ]]; then
      id -nG "$user" | tr ' ' '\n' | grep -qx aletheon || {
        echo "authorized user is not a member of the aletheon group: $user" >&2; exit 1;
      }
      [[ -d "/run/user/$uid" && -S "/run/user/$uid/bus" ]] || {
        echo "authorized user manager is unavailable: $user" >&2; exit 1;
      }
    fi
    state="$home/.local/state/aletheon"
    [[ -d "$state" && ! -L "$state" ]] || { echo "authorized user state is unavailable: $user" >&2; exit 1; }
    authorized_users+=("$user")
    authorized_uids+=("$uid")
    authorized_homes+=("$home")
  done < <(awk '{sub(/#.*/, ""); gsub(/^[[:space:]]+|[[:space:]]+$/, ""); if (length) print}' "$authorized_users_file")
  ((${#authorized_users[@]} > 0)) || { echo "authorized-user manifest is empty" >&2; exit 1; }
else
  [[ -z "$authorized_users_file" && -z "$user_backup_command" ]] || {
    echo "authorized-user options are not valid with --legacy-single-daemon" >&2; exit 64;
  }
fi

"$preflight" --preflight --binary "$binary" --config "$config"

# Both the machine authority state and every explicitly authorized user's state
# must have completed backup evidence before intake is stopped.
"$backup_command"
user_backup_receipt=
user_backup_receipt_sha256=
if [[ "$topology" == multi-user ]]; then
  [[ ! -e "$user_backup_root" && ! -L "$user_backup_root" ]] || {
    echo "refusing to reuse an existing user-state backup root" >&2; exit 1;
  }
  if [[ "$test_mode" == 1 ]]; then
    install -d -m 0700 "$user_backup_root"
  else
    install -d -o root -g root -m 0700 "$user_backup_root"
  fi
  "$user_backup_command" --output "$user_backup_root" --users-file "$authorized_users_file"
  user_backup_receipt="$user_backup_root/receipt.json"
  [[ -f "$user_backup_receipt" && ! -L "$user_backup_receipt" ]] || {
    echo "user-state backup produced no safe receipt" >&2; exit 1;
  }
  users_json=$(printf '%s\n' "${authorized_users[@]}" | jq -R . | jq -s .)
  jq -e --arg artifact_root "$user_backup_root" --argjson users "$users_json" '
    .schema_version == 1 and .status == "complete" and
    .rollback_required == true and .artifact_root == $artifact_root and
    .users == $users and (.completed_utc | type == "string" and length > 0)
  ' "$user_backup_receipt" >/dev/null || {
    echo "user-state backup receipt is invalid or incomplete" >&2; exit 1;
  }
  user_backup_receipt_sha256=$(sha256sum -- "$user_backup_receipt" | cut -d' ' -f1)
fi

current="$prefix/bin/aletheon"
pre_binary_sha256=
if [[ -x "$current" ]]; then
  if [[ "$test_mode" == 1 ]]; then
    install -m 0755 "$current" "$release_root/aletheon.pre-$timestamp"
  else
    install -o root -g root -m 0755 "$current" "$release_root/aletheon.pre-$timestamp"
  fi
  sha256sum "$release_root/aletheon.pre-$timestamp" >"$release_root/aletheon.pre-$timestamp.sha256"
  pre_binary_sha256=$(cut -d' ' -f1 "$release_root/aletheon.pre-$timestamp.sha256")
fi

user_systemctl() {
  local index=$1
  shift
  "$runuser_command" -u "${authorized_users[$index]}" -- env \
    "HOME=${authorized_homes[$index]}" \
    "XDG_RUNTIME_DIR=/run/user/${authorized_uids[$index]}" \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${authorized_uids[$index]}/bus" \
    "$systemctl_command" --user "$@"
}

stop_runtime() {
  local index
  if [[ "$topology" == legacy ]]; then
    "$systemctl_command" stop "$legacy_unit"
    return
  fi
  for index in "${!authorized_users[@]}"; do
    user_systemctl "$index" stop aletheon.socket aletheon.service
  done
  "$systemctl_command" stop "$core_unit"
}

start_runtime() {
  local index
  if [[ "$topology" == legacy ]]; then
    "$systemctl_command" start "$legacy_unit"
    return
  fi
  "$systemctl_command" start "$core_unit"
  for index in "${!authorized_users[@]}"; do
    user_systemctl "$index" start aletheon.socket
  done
}

stop_after_failed_readiness() {
  local index
  if [[ "$topology" == legacy ]]; then
    "$systemctl_command" stop "$legacy_unit" || true
    return
  fi
  for index in "${!authorized_users[@]}"; do
    user_systemctl "$index" stop aletheon.socket aletheon.service || true
  done
  "$systemctl_command" stop "$core_unit" || true
}

# Stop intake only after verified machine and user backups and a verified
# candidate are available.
stop_runtime
tmp="$prefix/bin/.aletheon.upgrade.$timestamp"
trap 'rm -f -- "${tmp:-}"' EXIT
if [[ "$test_mode" == 1 ]]; then
  install -m 0755 "$binary" "$tmp"
else
  install -o root -g root -m 0755 "$binary" "$tmp"
fi
mv -fT -- "$tmp" "$current"
trap - EXIT
"$preflight" --preflight --binary "$current" --config "$config"

# Database migrations are forward-only and run during daemon initialization.
# A failed start is intentionally not followed by blindly executing the old
# binary against possibly migrated databases.
if ! start_runtime; then
  stop_after_failed_readiness
  echo "upgrade start failed; keep all runtimes stopped and restore matching machine/user backups before using the saved binary" >&2
  exit 1
fi
if [[ "$topology" == legacy ]]; then
  if ! "$healthcheck" --user-socket "$legacy_socket"; then
    stop_after_failed_readiness
    echo "upgrade readiness failed; restore matching data before binary rollback" >&2
    exit 1
  fi
else
  if ! "$systemctl_command" is-active --quiet "$core_unit" || \
     ! "$healthcheck" --core-socket "$core_socket" || \
     ! "$healthcheck" --authorized-users "$authorized_users_file"; then
    stop_after_failed_readiness
    echo "upgrade readiness failed; restore matching machine/user data before binary rollback" >&2
    exit 1
  fi
fi

receipt="$receipt_root/$timestamp.json"
receipt_users=$(if [[ "$topology" == multi-user ]]; then printf '%s\n' "${authorized_users[@]}" | jq -R . | jq -s .; else printf '[]\n'; fi)
jq -n --arg installed_utc "$timestamp" --arg version "$version" --arg sha256 "$actual" \
  --arg pre_binary_sha256 "$pre_binary_sha256" --arg topology "$topology" \
  --arg core_unit "$core_unit" --arg user_backup_receipt "$user_backup_receipt" \
  --arg user_backup_receipt_sha256 "$user_backup_receipt_sha256" --argjson authorized_users "$receipt_users" \
  --arg config_sha256 "$(sha256sum -- "$config" | cut -d' ' -f1)" \
  '{schema_version:1,installed_utc:$installed_utc,version:$version,binary_sha256:$sha256,
    pre_binary_sha256:$pre_binary_sha256,config_sha256:$config_sha256,outcome:"ready",
    topology:$topology,core_unit:$core_unit,authorized_users:$authorized_users,
    user_backup_receipt:$user_backup_receipt,user_backup_receipt_sha256:$user_backup_receipt_sha256}' >"$receipt.tmp"
[[ "$test_mode" == 1 ]] || chown root:aletheon "$receipt.tmp"
chmod 0640 "$receipt.tmp"
mv -T -- "$receipt.tmp" "$receipt"
echo "upgrade complete and ready: receipt=$receipt" >&2
