#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
# shellcheck source=tests/production/lib/installed_host.sh
source "$repo_root/tests/production/lib/installed_host.sh"
require_disposable_installed_host
installed_test_users >/dev/null
export ALETHEON_TEST_USER_A ALETHEON_TEST_USER_B
artifacts=$(init_release_artifacts)
candidate=${ALETHEON_RELEASE_BINARY:-"$repo_root/target/release/aletheon"}
[[ -x "$candidate" && ! -L "$candidate" ]] || {
  echo "BLOCKED: build and supply the real release binary with ALETHEON_RELEASE_BINARY" >&2; exit 78;
}
baseline=${ALETHEON_BASELINE_BINARY:-}
[[ -n "$baseline" && -x "$baseline" && ! -L "$baseline" ]] || {
  echo "BLOCKED: supply the previous released binary with ALETHEON_BASELINE_BINARY" >&2; exit 78;
}
baseline_sha256=$(sha256sum "$baseline" | cut -d' ' -f1)
candidate_sha256=$(sha256sum "$candidate" | cut -d' ' -f1)
[[ "$baseline_sha256" != "$candidate_sha256" ]] || {
  echo "BLOCKED: baseline and candidate release binaries must be distinct" >&2; exit 78;
}

# The install script and checked-in units are the release assets under test. This
# script is intentionally impossible to run on an ordinary development host.
ALETHEON_BINARY="$baseline" ALETHEON_CONFIG="$repo_root/config/production.toml.example" \
  "$repo_root/scripts/libexec/aletheon/install-systemd.sh" --no-enable
prepare_installed_test_users
start_installed_runtime
assert_installed_readiness
assert_installed_boundaries "$artifacts"
install -d -m 0700 "$artifacts/install-journal"
capture_installed_journal "$artifacts/install-journal"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/install-integrity.txt"
capture_installed_user_integrity "$artifacts/install-user-integrity.txt"

# A controlled restart must preserve readiness and durable database integrity.
restart_installed_runtime
assert_installed_readiness
assert_installed_boundaries "$artifacts"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/restart-integrity.txt"
capture_installed_user_integrity "$artifacts/restart-user-integrity.txt"

# Preserve matching data before invoking the real upgrade asset. Staging mode is
# deliberately unencrypted and is valid only inside this disposable release drill.
bootstrap_backup="$artifacts/bootstrap-backup"
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT="$bootstrap_backup" \
  ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \
  ALETHEON_SCHEMA_VERSION="$(sha256sum "$repo_root/config/release/migration-matrix.toml" | cut -d' ' -f1)" \
  "$repo_root/scripts/libexec/aletheon/backup.sh"
# Health uses marker age while the manifest remains the authoritative backup
# receipt. Publish the marker only after the staging backup and integrity checks.
install -d -m 0750 /var/lib/aletheon/state
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg manifest_sha256 "$(sha256sum "$bootstrap_backup/manifest.json" | cut -d' ' -f1)" \
  '{completed_utc:$completed_utc,mode:"disposable_staging",manifest_sha256:$manifest_sha256}' \
  >/var/lib/aletheon/state/backup-marker.json
chmod 0640 /var/lib/aletheon/state/backup-marker.json
backup="$artifacts/pre-upgrade-backup"
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT="$backup" \
  ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \
  ALETHEON_SCHEMA_VERSION="$(sha256sum "$repo_root/config/release/migration-matrix.toml" | cut -d' ' -f1)" \
  "$repo_root/scripts/libexec/aletheon/backup.sh"
user_backup="$artifacts/pre-upgrade-user-state"
printf '%s  %s\n' "$baseline_sha256" "$baseline" >"$artifacts/baseline.sha256"
printf '%s  %s\n' "$candidate_sha256" "$candidate" >"$artifacts/candidate.sha256"
authorized_users="$artifacts/authorized-users.txt"
installed_test_users >"$authorized_users"
chmod 0600 "$authorized_users"
cat >"$artifacts/upgrade-backup.sh" <<UPGRADE_BACKUP
#!/usr/bin/env bash
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT='$artifacts/upgrade-script-backup' \\
ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \\
'$repo_root/scripts/libexec/aletheon/backup.sh'
UPGRADE_BACKUP
chmod 0700 "$artifacts/upgrade-backup.sh"
cat >"$artifacts/upgrade-user-backup.sh" <<UPGRADE_USER_BACKUP
#!/usr/bin/env bash
set -euo pipefail
source '$repo_root/tests/production/lib/installed_host.sh'
output=
users_file=
while ((\$#)); do
  case "\$1" in
    --output) output=\${2:-}; shift 2 ;;
    --users-file) users_file=\${2:-}; shift 2 ;;
    *) exit 64 ;;
  esac
done
[[ -n "\$output" && -f "\$users_file" ]] || exit 64
cmp -s "\$users_file" '$authorized_users' || {
  echo 'upgrade passed an unexpected authorized-user manifest' >&2; exit 1;
}
backup_installed_user_state "\$output"
users_json=\$(jq -R . "\$users_file" | jq -s .)
jq -n --arg completed_utc "\$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg artifact_root "\$output" --argjson users "\$users_json" \
  '{schema_version:1,status:"complete",rollback_required:true,completed_utc:\$completed_utc,artifact_root:\$artifact_root,users:\$users}' \
  >"\$output/receipt.json"
UPGRADE_USER_BACKUP
chmod 0700 "$artifacts/upgrade-user-backup.sh"
ALETHEON_BACKUP_COMMAND="$artifacts/upgrade-backup.sh" \
ALETHEON_PREFLIGHT_COMMAND=/usr/libexec/aletheon/verify-systemd.sh \
ALETHEON_USER_BACKUP_ROOT="$user_backup" \
  /usr/libexec/aletheon/upgrade-aletheon.sh --binary "$candidate" \
    --sha256-file "$artifacts/candidate.sha256" --config /etc/aletheon/config.toml \
    --authorized-users "$authorized_users" \
    --user-backup-command "$artifacts/upgrade-user-backup.sh"
assert_installed_readiness
assert_installed_boundaries "$artifacts"
cp -a /var/lib/aletheon/state/upgrades "$artifacts/upgrade-receipts"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/upgrade-integrity.txt"
capture_installed_user_integrity "$artifacts/upgrade-user-integrity.txt"

# Any data migration requires a matching data+binary rollback. Restore to empty
# roots first, retain upgraded state as evidence, then place both matching parts.
stop_installed_runtime
mv /var/lib/aletheon "$artifacts/upgraded-data-root"
archive_installed_user_state "$artifacts/upgraded-user-state"
ALETHEON_RESTORE_SOURCE="$backup" ALETHEON_RESTORE_TARGET=/var/lib/aletheon \
  ALETHEON_RESTORE_CONFIG_TARGET="$artifacts/restored-config" \
  "$repo_root/scripts/libexec/aletheon/restore.sh"
saved=$(find "$artifacts/upgraded-data-root/releases" -maxdepth 1 -type f -name 'aletheon.pre-*' ! -name '*.sha256' | sort | tail -n1)
[[ -x "$saved" ]] || { echo "matching pre-upgrade binary was not preserved" >&2; exit 1; }
install -m 0755 "$saved" /usr/bin/aletheon
restore_installed_user_state "$user_backup"
start_installed_runtime
assert_installed_readiness
assert_installed_boundaries "$artifacts"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/rollback-integrity.txt"
capture_installed_user_integrity "$artifacts/rollback-user-integrity.txt"
install -d -m 0700 "$artifacts/final-journal"
capture_installed_journal "$artifacts/final-journal"

# The rollback drill must not leave the aggregate production scenarios running
# against the baseline release. Reapply the candidate from the matching restored
# baseline state and prove that the installed executable is the requested build.
cat >"$artifacts/final-upgrade-backup.sh" <<FINAL_UPGRADE_BACKUP
#!/usr/bin/env bash
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT='$artifacts/final-upgrade-script-backup' \
ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \
'$repo_root/scripts/libexec/aletheon/backup.sh'
FINAL_UPGRADE_BACKUP
chmod 0700 "$artifacts/final-upgrade-backup.sh"
final_user_backup="$artifacts/final-upgrade-user-state"
ALETHEON_BACKUP_COMMAND="$artifacts/final-upgrade-backup.sh" \
ALETHEON_PREFLIGHT_COMMAND=/usr/libexec/aletheon/verify-systemd.sh \
ALETHEON_USER_BACKUP_ROOT="$final_user_backup" \
  /usr/libexec/aletheon/upgrade-aletheon.sh --binary "$candidate" \
    --sha256-file "$artifacts/candidate.sha256" --config /etc/aletheon/config.toml \
    --authorized-users "$authorized_users" \
    --user-backup-command "$artifacts/upgrade-user-backup.sh"
[[ $(sha256sum /usr/bin/aletheon | cut -d' ' -f1) == "$candidate_sha256" ]] || {
  echo "final installed executable does not match the candidate" >&2; exit 1;
}
assert_installed_readiness
assert_installed_boundaries "$artifacts"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/final-candidate-integrity.txt"
capture_installed_user_integrity "$artifacts/final-candidate-user-integrity.txt"

jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg artifacts "$artifacts" --arg baseline_sha256 "$baseline_sha256" \
  --arg candidate_sha256 "$candidate_sha256" \
  --arg user_a "$ALETHEON_TEST_USER_A" --arg user_b "$ALETHEON_TEST_USER_B" \
  '{status:"PASS",lane:"disposable-installed-host",completed_utc:$completed_utc,artifacts:$artifacts,baseline_sha256:$baseline_sha256,candidate_sha256:$candidate_sha256,active_binary_sha256:$candidate_sha256,distinct_release_upgrade:($baseline_sha256 != $candidate_sha256),rollback:"matching_system_user_state_and_binary",post_rollback_candidate_reapplied:true,system_unit:"aletheon-core.service",user_state_root:"$HOME/.local/state/aletheon",user_socket_activation:true,test_users:[$user_a,$user_b]}' \
  >"$artifacts/operator-receipt.json"
echo "installed-host release drill passed: $artifacts/operator-receipt.json"
