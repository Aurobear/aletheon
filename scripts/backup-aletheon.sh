#!/usr/bin/env bash
set -euo pipefail

data_root=${ALETHEON_DATA_ROOT:-/var/lib/aletheon}
config_root=${ALETHEON_CONFIG_ROOT:-/etc/aletheon}
cache_root=${ALETHEON_BACKUP_CACHE:-/var/cache/aletheon/backup}
password_file=${RESTIC_PASSWORD_FILE:-/etc/aletheon/credentials/restic-password}
repository_file=${RESTIC_REPOSITORY_FILE:-/etc/aletheon/credentials/restic-repository}
host_id=${ALETHEON_HOST_ID:-$(cat /etc/machine-id 2>/dev/null || hostname)}
schema_version=${ALETHEON_SCHEMA_VERSION:-unknown}
aletheon_version=${ALETHEON_VERSION:-unknown}
mode=${ALETHEON_BACKUP_MODE:-restic}

for command in sqlite3 jq sha256sum tar find; do command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 1; }; done
[[ -d "$data_root" && ! -L "$data_root" ]] || { echo "invalid data root" >&2; exit 1; }
[[ -d "$config_root" && ! -L "$config_root" ]] || { echo "invalid config root" >&2; exit 1; }
umask 077
install -d -m 0700 "$cache_root"
stage=$(mktemp -d --tmpdir="$cache_root" snapshot.XXXXXX)
trap 'rm -rf -- "$stage"' EXIT
install -d -m 0700 "$stage/data" "$stage/config" "$stage/sqlite"

# Copy managed non-database state. SQLite files are captured separately through
# the online backup API so a live WAL cannot be copied inconsistently.
tar -C "$data_root" --exclude='*.db' --exclude='*.db-wal' --exclude='*.db-shm' \
  --exclude='*.sqlite' --exclude='*.sqlite-wal' --exclude='*.sqlite-shm' \
  --exclude='backup' -cf - . | tar -C "$stage/data" -xf -
# Credential values and vault recovery keys remain in a separately encrypted
# recovery system. The primary repository contains policy/config references and
# the encrypted vault payload, never `/etc/aletheon/credentials` itself.
tar -C "$config_root" --exclude='credentials' -cf - . | tar -C "$stage/config" -xf -

db_count=0
while IFS= read -r -d '' db; do
  rel=${db#"$data_root"/}
  dest="$stage/sqlite/$rel"
  install -d -m 0700 "$(dirname -- "$dest")"
  case "$dest" in *"'"*) echo "unsupported quote in database path" >&2; exit 1 ;; esac
  sqlite3 "$db" ".timeout 30000" ".backup '$dest'"
  [[ $(sqlite3 "$dest" 'PRAGMA integrity_check;') == ok ]] || { echo "SQLite backup integrity failed: $rel" >&2; exit 1; }
  db_count=$((db_count + 1))
done < <(find "$data_root" -xdev -type f \( -name '*.db' -o -name '*.sqlite' \) -print0)

created=$(date -u +%Y-%m-%dT%H:%M:%SZ)
files_json="$stage/.files.json"
: >"$files_json"
while IFS= read -r -d '' file; do
  rel=${file#"$stage"/}
  hash=$(sha256sum -- "$file" | cut -d' ' -f1)
  jq -cn --arg path "$rel" --arg sha256 "$hash" --argjson bytes "$(stat -Lc '%s' "$file")" \
    '{path:$path,sha256:$sha256,bytes:$bytes}' >>"$files_json"
done < <(find "$stage/data" "$stage/config" "$stage/sqlite" -type f -print0 | sort -z)
jq -s --arg created "$created" --arg host "$host_id" --arg version "$aletheon_version" \
  --arg schema "$schema_version" --argjson db_count "$db_count" \
  '{format_version:1,aletheon_version:$version,schema_version:$schema,created_utc:$created,host_id:$host,components:["goal-approval-channel-google-state","mnemosyne-gbrain","artifacts","audit","config-policy","encrypted-credential-vault"],sqlite_databases:$db_count,repository_snapshot_id:null,files:.}' \
  "$files_json" >"$stage/manifest.json"
rm -f -- "$files_json"

if [[ "$mode" == staging ]]; then
  destination=${ALETHEON_BACKUP_OUTPUT:?ALETHEON_BACKUP_OUTPUT is required in staging mode}
  [[ ! -e "$destination" ]] || { echo "staging destination already exists" >&2; exit 1; }
  mv -- "$stage" "$destination"
  trap - EXIT
  echo "backup staged: files=$(jq '.files|length' "$destination/manifest.json") databases=$db_count" >&2
  exit 0
fi

command -v restic >/dev/null || { echo "missing command: restic" >&2; exit 1; }
[[ -r "$password_file" && -r "$repository_file" ]] || { echo "backup credential files are unreadable" >&2; exit 1; }
export RESTIC_PASSWORD_FILE="$password_file"
snapshot=$(restic --repository-file "$repository_file" backup --json --tag aletheon-data "$stage" \
  | jq -r 'select(.message_type=="summary") | .snapshot_id' | tail -n1)
[[ "$snapshot" =~ ^[0-9a-f]+$ ]] || { echo "restic did not return a snapshot id" >&2; exit 1; }
tmp_manifest="$stage/manifest.json.new"
jq --arg snapshot "$snapshot" '.repository_snapshot_id=$snapshot' "$stage/manifest.json" >"$tmp_manifest"
mv -T -- "$tmp_manifest" "$stage/manifest.json"
receipt=$(restic --repository-file "$repository_file" backup --json --tag aletheon-manifest "$stage/manifest.json" \
  | jq -r 'select(.message_type=="summary") | .snapshot_id' | tail -n1)
restic --repository-file "$repository_file" check --read-data-subset=1/20 >/dev/null
marker="$data_root/state/backup-marker.json"
install -d -m 0750 "$(dirname -- "$marker")"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg snapshot "$snapshot" --arg receipt "$receipt" \
  '{completed_utc:$completed_utc,snapshot_id:$snapshot,receipt_snapshot_id:$receipt}' >"$marker.tmp"
chmod 0640 "$marker.tmp"
mv -T -- "$marker.tmp" "$marker"
echo "backup complete: snapshot=$snapshot receipt=$receipt files=$(jq '.files|length' "$stage/manifest.json") databases=$db_count" >&2
