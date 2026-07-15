#!/usr/bin/env bash
set -euo pipefail

target=${ALETHEON_RESTORE_TARGET:-/var/lib/aletheon}
config_target=${ALETHEON_RESTORE_CONFIG_TARGET:-/etc/aletheon.restore}
source_dir=${ALETHEON_RESTORE_SOURCE:-}
password_file=${RESTIC_PASSWORD_FILE:-/etc/aletheon/credentials/restic-password}
repository_file=${RESTIC_REPOSITORY_FILE:-/etc/aletheon/credentials/restic-repository}
snapshot=${ALETHEON_RESTORE_SNAPSHOT:-latest}
expected_schema=${ALETHEON_SCHEMA_VERSION:-}

for command in sqlite3 jq sha256sum find; do command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 1; }; done
case "$target" in /|/var|/var/lib) echo "refusing unsafe restore target" >&2; exit 1 ;; esac
if [[ -e "$target" ]] && find "$target" -mindepth 1 -print -quit | grep -q .; then
  echo "restore target is not empty; restore to a new staging directory" >&2; exit 1
fi
if [[ -e "$config_target" ]] && find "$config_target" -mindepth 1 -print -quit | grep -q .; then
  echo "restore config target is not empty; restore to a new staging directory" >&2; exit 1
fi
umask 077
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
if [[ -n "$source_dir" ]]; then
  [[ -d "$source_dir" && ! -L "$source_dir" ]] || { echo "invalid restore source" >&2; exit 1; }
  cp -a -- "$source_dir/." "$work/"
else
  command -v restic >/dev/null || { echo "missing command: restic" >&2; exit 1; }
  [[ -r "$password_file" && -r "$repository_file" ]] || { echo "backup credential files are unreadable" >&2; exit 1; }
  export RESTIC_PASSWORD_FILE="$password_file"
  restic --repository-file "$repository_file" restore "$snapshot" --target "$work"
  stage=$(find "$work" -type f -name manifest.json -printf '%h\n' | head -n1)
  [[ -n "$stage" ]] || { echo "snapshot contains no manifest" >&2; exit 1; }
  work=$stage
fi

manifest="$work/manifest.json"
jq -e '.format_version==1 and (.files|type=="array")' "$manifest" >/dev/null || { echo "invalid backup manifest" >&2; exit 1; }
if [[ -n "$expected_schema" ]]; then
  [[ $(jq -r '.schema_version' "$manifest") == "$expected_schema" ]] || { echo "backup schema version is incompatible" >&2; exit 1; }
fi
while IFS=$'\t' read -r rel expected; do
  [[ "$rel" != /* && "$rel" != *'..'* && -f "$work/$rel" && ! -L "$work/$rel" ]] || { echo "unsafe or missing manifest path" >&2; exit 1; }
  actual=$(sha256sum -- "$work/$rel" | cut -d' ' -f1)
  [[ "$actual" == "$expected" ]] || { echo "backup hash mismatch: $rel" >&2; exit 1; }
done < <(jq -r '.files[] | [.path,.sha256] | @tsv' "$manifest")
while IFS= read -r -d '' db; do
  [[ $(sqlite3 "$db" 'PRAGMA integrity_check;') == ok ]] || { echo "SQLite integrity failed before restore" >&2; exit 1; }
done < <(find "$work/sqlite" -type f \( -name '*.db' -o -name '*.sqlite' \) -print0)

install -d -m 0750 "$target"
install -d -m 0750 "$config_target"
cp -a -- "$work/data/." "$target/"
cp -a -- "$work/config/." "$config_target/"
while IFS= read -r -d '' db; do
  rel=${db#"$work/sqlite"/}
  install -d -m 0750 "$target/$(dirname -- "$rel")"
  install -m 0600 "$db" "$target/$rel"
done < <(find "$work/sqlite" -type f \( -name '*.db' -o -name '*.sqlite' \) -print0)
find "$target" -type d -exec chmod go-w {} +
find "$target" -type f -exec chmod go-w {} +
if [[ ${EUID:-$(id -u)} -eq 0 ]] && id -u aletheon >/dev/null 2>&1; then
  chown -R aletheon:aletheon "$target"
  chown -R root:aletheon "$config_target"
  find "$config_target" -type d -exec chmod 0750 {} +
  find "$config_target" -type f -exec chmod 0640 {} +
fi
while IFS= read -r -d '' db; do
  [[ $(sqlite3 "$db" 'PRAGMA integrity_check;') == ok ]] || { echo "restored SQLite integrity failed" >&2; exit 1; }
done < <(find "$target" -type f \( -name '*.db' -o -name '*.sqlite' \) -print0)
echo "restore staged and verified: target=$target config_target=$config_target files=$(jq '.files|length' "$manifest")" >&2
