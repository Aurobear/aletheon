#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --binary FILE --sha256-file FILE [--config FILE] [--assets DIR]" >&2
  exit 64
}

test_mode=${ALETHEON_UNPRIVILEGED_TEST:-0}
[[ ${EUID:-$(id -u)} -eq 0 || "$test_mode" == 1 ]] || { echo "run as root" >&2; exit 1; }
binary=
sha_file=
config=/etc/aletheon/config.toml
assets=
while (($#)); do
  case "$1" in
    --binary) binary=${2:-}; shift 2 ;;
    --sha256-file) sha_file=${2:-}; shift 2 ;;
    --config) config=${2:-}; shift 2 ;;
    --assets) assets=${2:-}; shift 2 ;;
    *) usage ;;
  esac
done
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
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
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

[[ -x "$backup_command" ]] || { echo "backup command is unavailable" >&2; exit 1; }
"$backup_command"

current="$prefix/bin/aletheon"
if [[ -x "$current" ]]; then
  if [[ "$test_mode" == 1 ]]; then
    install -m 0755 "$current" "$release_root/aletheon.pre-$timestamp"
  else
    install -o root -g root -m 0755 "$current" "$release_root/aletheon.pre-$timestamp"
  fi
  sha256sum "$release_root/aletheon.pre-$timestamp" >"$release_root/aletheon.pre-$timestamp.sha256"
fi

# Stop intake only after a verified backup and candidate are available.
"$systemctl_command" stop aletheon.service
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
if ! "$systemctl_command" start aletheon.service; then
  echo "upgrade start failed; keep service stopped and restore the pre-upgrade backup before using the saved binary" >&2
  exit 1
fi
if ! "$healthcheck" /run/aletheon/aletheon.sock; then
  "$systemctl_command" stop aletheon.service || true
  echo "upgrade readiness failed; restore matching pre-upgrade data before binary rollback" >&2
  exit 1
fi

receipt="$receipt_root/$timestamp.json"
jq -n --arg installed_utc "$timestamp" --arg version "$version" --arg sha256 "$actual" \
  --arg config_sha256 "$(sha256sum -- "$config" | cut -d' ' -f1)" \
  '{installed_utc:$installed_utc,version:$version,binary_sha256:$sha256,config_sha256:$config_sha256,outcome:"ready"}' >"$receipt.tmp"
[[ "$test_mode" == 1 ]] || chown root:aletheon "$receipt.tmp"
chmod 0640 "$receipt.tmp"
mv -T -- "$receipt.tmp" "$receipt"
echo "upgrade complete and ready: receipt=$receipt" >&2
