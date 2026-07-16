#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
# shellcheck source=tests/production/lib/installed_host.sh
source "$repo_root/tests/production/lib/installed_host.sh"
require_disposable_installed_host
artifacts=$(init_release_artifacts)
candidate=${ALETHEON_RELEASE_BINARY:-"$repo_root/target/release/aletheon"}
[[ -x "$candidate" && ! -L "$candidate" ]] || {
  echo "BLOCKED: build and supply the real release binary with ALETHEON_RELEASE_BINARY" >&2; exit 78;
}

# The install script and checked-in units are the release assets under test. This
# script is intentionally impossible to run on an ordinary development host.
ALETHEON_BINARY="$candidate" ALETHEON_CONFIG="$repo_root/config/production.toml.example" \
  "$repo_root/scripts/install-systemd.sh" --no-enable
systemctl start aletheon.service
"$repo_root/scripts/verify-systemd.sh" --readiness --socket /run/aletheon/aletheon.sock --timeout 30
assert_installed_boundaries "$artifacts"
journalctl -u aletheon.service --no-pager -n 300 >"$artifacts/install-journal.txt"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/install-integrity.txt"

# A controlled restart must preserve readiness and durable database integrity.
systemctl restart aletheon.service
"$repo_root/scripts/verify-systemd.sh" --readiness --socket /run/aletheon/aletheon.sock --timeout 30
capture_sqlite_integrity /var/lib/aletheon "$artifacts/restart-integrity.txt"

# Preserve matching data before invoking the real upgrade asset. Staging mode is
# deliberately unencrypted and is valid only inside this disposable release drill.
bootstrap_backup="$artifacts/bootstrap-backup"
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT="$bootstrap_backup" \
  ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \
  ALETHEON_SCHEMA_VERSION="$(sha256sum "$repo_root/config/release/migration-matrix.toml" | cut -d' ' -f1)" \
  "$repo_root/scripts/backup-aletheon.sh"
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
  "$repo_root/scripts/backup-aletheon.sh"
sha256sum "$candidate" >"$artifacts/candidate.sha256"
cat >"$artifacts/upgrade-backup.sh" <<UPGRADE_BACKUP
#!/usr/bin/env bash
ALETHEON_BACKUP_MODE=staging ALETHEON_BACKUP_OUTPUT='$artifacts/upgrade-script-backup' \\
ALETHEON_DATA_ROOT=/var/lib/aletheon ALETHEON_CONFIG_ROOT=/etc/aletheon \\
'$repo_root/scripts/backup-aletheon.sh'
UPGRADE_BACKUP
chmod 0700 "$artifacts/upgrade-backup.sh"
ALETHEON_BACKUP_COMMAND="$artifacts/upgrade-backup.sh" \
ALETHEON_PREFLIGHT_COMMAND=/usr/libexec/aletheon/verify-systemd.sh \
ALETHEON_HEALTHCHECK_COMMAND=/usr/libexec/aletheon/aletheon-healthcheck.sh \
  /usr/libexec/aletheon/upgrade-aletheon.sh --binary "$candidate" \
    --sha256-file "$artifacts/candidate.sha256" --config /etc/aletheon/config.toml
cp -a /var/lib/aletheon/state/upgrades "$artifacts/upgrade-receipts"
capture_sqlite_integrity /var/lib/aletheon "$artifacts/upgrade-integrity.txt"

# Any data migration requires a matching data+binary rollback. Restore to empty
# roots first, retain upgraded state as evidence, then place both matching parts.
systemctl stop aletheon.service
mv /var/lib/aletheon "$artifacts/upgraded-data-root"
ALETHEON_RESTORE_SOURCE="$backup" ALETHEON_RESTORE_TARGET=/var/lib/aletheon \
  ALETHEON_RESTORE_CONFIG_TARGET="$artifacts/restored-config" \
  "$repo_root/scripts/restore-aletheon.sh"
saved=$(find "$artifacts/upgraded-data-root/releases" -maxdepth 1 -type f -name 'aletheon.pre-*' ! -name '*.sha256' | sort | tail -n1)
[[ -x "$saved" ]] || { echo "matching pre-upgrade binary was not preserved" >&2; exit 1; }
install -m 0755 "$saved" /usr/bin/aletheon
systemctl start aletheon.service
"$repo_root/scripts/verify-systemd.sh" --readiness --socket /run/aletheon/aletheon.sock --timeout 30
capture_sqlite_integrity /var/lib/aletheon "$artifacts/rollback-integrity.txt"
journalctl -u aletheon.service --no-pager >"$artifacts/final-journal.txt"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg artifacts "$artifacts" --arg candidate_sha256 "$(cut -d' ' -f1 "$artifacts/candidate.sha256")" \
  '{status:"PASS",lane:"disposable-installed-host",completed_utc:$completed_utc,artifacts:$artifacts,candidate_sha256:$candidate_sha256,rollback:"matching_data_and_binary"}' \
  >"$artifacts/operator-receipt.json"
echo "installed-host release drill passed: $artifacts/operator-receipt.json"
