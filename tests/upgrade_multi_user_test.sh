#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
tmp=$(mktemp -d /tmp/aletheon-upgrade-multi-user.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/prefix/bin" "$tmp/home/.local/state/aletheon"
printf '[runtime]\n' >"$tmp/config.toml"
printf 'testuser\n' >"$tmp/users.txt"
chmod 0600 "$tmp/users.txt"

cat >"$tmp/candidate" <<'EOF'
#!/usr/bin/env bash
[[ ${1-} == version ]] && { echo 'aletheon test-candidate'; exit 0; }
exit 64
EOF
cat >"$tmp/prefix/bin/aletheon" <<'EOF'
#!/usr/bin/env bash
[[ ${1-} == version ]] && { echo 'aletheon old'; exit 0; }
exit 64
EOF
chmod 0755 "$tmp/candidate" "$tmp/prefix/bin/aletheon"
sha256sum "$tmp/candidate" >"$tmp/candidate.sha256"

cat >"$tmp/bin/getent" <<EOF
#!/usr/bin/env bash
[[ \${1-}:\${2-} == passwd:testuser ]] || exit 2
echo 'testuser:x:12345:12345::${tmp}/home:/bin/sh'
EOF
cat >"$tmp/bin/preflight" <<EOF
#!/usr/bin/env bash
printf 'preflight:%s\n' "\$*" >>'$tmp/events'
EOF
cat >"$tmp/bin/machine-backup" <<EOF
#!/usr/bin/env bash
echo machine-backup >>'$tmp/events'
EOF
cat >"$tmp/bin/user-backup" <<EOF
#!/usr/bin/env bash
set -euo pipefail
output=
users_file=
while ((\$#)); do
  case "\$1" in
    --output) output=\$2; shift 2 ;;
    --users-file) users_file=\$2; shift 2 ;;
    *) exit 64 ;;
  esac
done
echo user-backup >>'$tmp/events'
users=\$(jq -R . "\$users_file" | jq -s .)
jq -n --arg completed_utc 2026-07-17T00:00:00Z --arg artifact_root "\$output" \
  --argjson users "\$users" \
  '{schema_version:1,status:"complete",rollback_required:true,completed_utc:\$completed_utc,artifact_root:\$artifact_root,users:\$users}' \
  >"\$output/receipt.json"
EOF
cat >"$tmp/bin/systemctl" <<EOF
#!/usr/bin/env bash
printf 'systemctl:%s\n' "\$*" >>'$tmp/events'
EOF
cat >"$tmp/bin/runuser" <<EOF
#!/usr/bin/env bash
printf 'runuser:%s\n' "\$*" >>'$tmp/events'
while [[ \$# -gt 0 && \$1 != -- ]]; do shift; done
shift
exec "\$@"
EOF
cat >"$tmp/bin/healthcheck" <<EOF
#!/usr/bin/env bash
printf 'health:%s\n' "\$*" >>'$tmp/events'
case "\${1-}:\${2-}" in
  --core-socket:$tmp/core.sock|--authorized-users:$tmp/users.txt) ;;
  *) exit 64 ;;
esac
EOF
chmod 0755 "$tmp/bin/"*

common_env=(
  PATH="$tmp/bin:$PATH"
  ALETHEON_UNPRIVILEGED_TEST=1
  ALETHEON_INSTALL_PREFIX="$tmp/prefix"
  ALETHEON_RELEASE_ROOT="$tmp/releases"
  ALETHEON_RECEIPT_ROOT="$tmp/receipts"
  ALETHEON_BACKUP_COMMAND="$tmp/bin/machine-backup"
  ALETHEON_PREFLIGHT_COMMAND="$tmp/bin/preflight"
  ALETHEON_HEALTHCHECK_COMMAND="$tmp/bin/healthcheck"
  ALETHEON_SYSTEMCTL_COMMAND="$tmp/bin/systemctl"
  ALETHEON_RUNUSER_COMMAND="$tmp/bin/runuser"
  ALETHEON_CORE_SOCKET="$tmp/core.sock"
  ALETHEON_USER_BACKUP_ROOT="$tmp/user-backup"
)

# The default topology must reject an implicit/partial user-state migration
# before invoking either backup command or stopping a runtime.
if env "${common_env[@]}" "$repo_root/scripts/upgrade-aletheon.sh" \
  --binary "$tmp/candidate" --sha256-file "$tmp/candidate.sha256" \
  --config "$tmp/config.toml" --authorized-users "$tmp/users.txt" \
  >"$tmp/missing.out" 2>&1; then
  echo "upgrade unexpectedly accepted a missing user backup command" >&2
  exit 1
fi
[[ ! -e "$tmp/events" ]]

env "${common_env[@]}" "$repo_root/scripts/upgrade-aletheon.sh" \
  --binary "$tmp/candidate" --sha256-file "$tmp/candidate.sha256" \
  --config "$tmp/config.toml" --authorized-users "$tmp/users.txt" \
  --user-backup-command "$tmp/bin/user-backup"

machine_line=$(grep -n '^machine-backup$' "$tmp/events" | cut -d: -f1)
user_line=$(grep -n '^user-backup$' "$tmp/events" | cut -d: -f1)
stop_line=$(grep -n 'runuser:.* stop aletheon.socket aletheon.service$' "$tmp/events" | cut -d: -f1)
[[ "$machine_line" -lt "$stop_line" && "$user_line" -lt "$stop_line" ]]
grep -q '^systemctl:stop aletheon-core.service$' "$tmp/events"
grep -q '^systemctl:start aletheon-core.service$' "$tmp/events"
grep -q 'runuser:.* start aletheon.socket$' "$tmp/events"
grep -q "^health:--core-socket $tmp/core.sock$" "$tmp/events"
grep -q "^health:--authorized-users $tmp/users.txt$" "$tmp/events"
! grep -q '^systemctl:stop aletheon.service$' "$tmp/events"

receipt=$(find "$tmp/receipts" -maxdepth 1 -type f -name '*.json' | head -n1)
jq -e --arg receipt "$tmp/user-backup/receipt.json" '
  .topology == "multi-user" and .core_unit == "aletheon-core.service" and
  .authorized_users == ["testuser"] and .user_backup_receipt == $receipt and
  (.user_backup_receipt_sha256 | length == 64) and .outcome == "ready"
' "$receipt" >/dev/null

echo "multi-user upgrade default-path test passed"
