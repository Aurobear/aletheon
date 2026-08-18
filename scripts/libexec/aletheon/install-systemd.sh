#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
binary=${ALETHEON_BINARY:-"$repo_root/target/release/aletheon"}
config_source=${ALETHEON_CONFIG:-"$repo_root/config/production.toml.example"}
user_config=${ALETHEON_USER_CONFIG:-}
enable=1
[[ ${1-} == --no-enable ]] && enable=0

[[ -x "$binary" ]] || { echo "missing executable: $binary" >&2; exit 1; }
[[ -f "$config_source" && ! -L "$config_source" ]] || {
  echo "missing or symlinked config: $config_source" >&2; exit 1;
}
getent group aletheon >/dev/null || groupadd --system aletheon
id -u aletheon >/dev/null 2>&1 || useradd --system --gid aletheon \
  --home-dir /var/lib/aletheon --shell /usr/sbin/nologin aletheon

install -d -o root -g aletheon -m 0750 /etc/aletheon /etc/aletheon/policy /etc/aletheon/credentials
install -d -o aletheon -g aletheon -m 0750 \
  /var/lib/aletheon/{state,goals,sessions,mnemosyne,artifacts,worktrees,audit} \
  /var/cache/aletheon /run/aletheon
deployment_id=$(date -u +%Y%m%dT%H%M%SZ)-$$
rollback_dir=/var/lib/aletheon/state/deployments/$deployment_id
install -d -o root -g aletheon -m 0750 "$rollback_dir"

backup_artifact() {
  local source=$1 fallback=$2 target=$3 mode=$4
  if [[ -n "$source" && -f "$source" && ! -L "$source" ]]; then
    install -o root -g aletheon -m "$mode" "$source" "$target"
  elif [[ -n "$fallback" ]]; then
    install -o root -g aletheon -m "$mode" "$fallback" "$target"
  else
    install -o root -g aletheon -m "$mode" /dev/null "$target"
  fi
}

previous_binary=$rollback_dir/aletheon
previous_core_config=$rollback_dir/core-config.toml
previous_user_config=$rollback_dir/user-config.toml
backup_artifact /usr/bin/aletheon "$binary" "$previous_binary" 0750
backup_artifact /etc/aletheon/config.toml "$config_source" "$previous_core_config" 0640
backup_artifact "$user_config" '' "$previous_user_config" 0640
install -d -o aletheon -g aletheon -m 0700 /var/cache/aletheon/backup
for secret in provider.env telegram.env gbrain.env; do
  if [[ ! -e /etc/aletheon/credentials/$secret ]]; then
    install -o aletheon -g aletheon -m 0600 /dev/null "/etc/aletheon/credentials/$secret"
  fi
done

install -o root -g root -m 0755 "$binary" /usr/bin/aletheon
bash "$repo_root/scripts/libexec/aletheon/install-completions.sh" --system /
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/verify/systemd.sh" \
  /usr/libexec/aletheon/verify-systemd.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/secret-audit.sh" \
  /usr/libexec/aletheon/aletheon-secret-audit.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/secret-init.sh" \
  /usr/libexec/aletheon/aletheon-secret-init.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/healthcheck.sh" \
  /usr/libexec/aletheon/aletheon-healthcheck.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/backup.sh" \
  /usr/libexec/aletheon/backup-aletheon.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/restore.sh" \
  /usr/libexec/aletheon/restore-aletheon.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/cleanup.sh" \
  /usr/libexec/aletheon/cleanup-aletheon.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/verify/network-exposure.sh" \
  /usr/libexec/aletheon/verify-network-exposure.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/libexec/aletheon/upgrade.sh" \
  /usr/libexec/aletheon/upgrade-aletheon.sh
/usr/libexec/aletheon/aletheon-secret-init.sh init /etc/aletheon/credentials
install -o root -g root -m 0644 "$repo_root/config/aletheon.service" \
  /etc/systemd/system/aletheon.service
install -o root -g root -m 0644 "$repo_root/config/aletheon-core.service" \
  /etc/systemd/system/aletheon-core.service
install -d -o root -g root -m 0755 /usr/lib/systemd/user
install -o root -g root -m 0644 "$repo_root/config/aletheon.user.service" \
  /usr/lib/systemd/user/aletheon.service
sed -i 's|ExecStart=%h/.local/bin/aletheon daemon|ExecStart=/usr/bin/aletheon daemon|' \
  /usr/lib/systemd/user/aletheon.service
install -o root -g root -m 0644 "$repo_root/config/aletheon-memory-agent.user.service" \
  /usr/lib/systemd/user/aletheon-memory-agent.service
sed -i 's|ExecStart=%h/.local/bin/aletheon memory-agent serve --official-user-socket|ExecStart=/usr/bin/aletheon memory-agent serve --official-user-socket|' \
  /usr/lib/systemd/user/aletheon-memory-agent.service
install -o root -g root -m 0644 "$repo_root/config/aletheon.user.socket" \
  /usr/lib/systemd/user/aletheon.socket
for unit in aletheon-backup.service aletheon-backup.timer \
  aletheon-cleanup.service aletheon-cleanup.timer; do
  install -o root -g root -m 0644 "$repo_root/config/$unit" "/etc/systemd/system/$unit"
done
install -o root -g root -m 0644 "$repo_root/config/aletheon.logrotate" \
  /etc/logrotate.d/aletheon
if [[ ! -e /etc/aletheon/config.toml ]]; then
  install -o root -g aletheon -m 0640 "$config_source" /etc/aletheon/config.toml
fi
install -D -o root -g root -m 0644 "$repo_root/docs/deployment/systemd.md" \
  /usr/share/doc/aletheon/systemd.md

/usr/libexec/aletheon/verify-systemd.sh --preflight \
  --binary /usr/bin/aletheon --config /etc/aletheon/config.toml
/usr/libexec/aletheon/verify-systemd.sh --core-unit \
  /etc/systemd/system/aletheon.service --binary /usr/bin/aletheon
/usr/libexec/aletheon/verify-systemd.sh --core-unit \
  /etc/systemd/system/aletheon-core.service --binary /usr/bin/aletheon
systemd-analyze verify /etc/systemd/system/aletheon-backup.service
systemd-analyze verify /etc/systemd/system/aletheon-cleanup.service
/usr/libexec/aletheon/verify-systemd.sh --user-units \
  /usr/lib/systemd/user/aletheon.service /usr/lib/systemd/user/aletheon.socket \
  /usr/lib/systemd/user/aletheon-memory-agent.service \
  --binary /usr/bin/aletheon
systemctl daemon-reload
if ((enable)); then
  systemctl disable --now aletheon.service
  systemctl enable aletheon-core.service
  systemctl reset-failed aletheon-core.service
  systemctl restart aletheon-core.service
  systemctl --global enable aletheon.socket aletheon-memory-agent.service
  systemctl enable --now aletheon-cleanup.timer
  if command -v restic >/dev/null \
    && [[ -s /etc/aletheon/credentials/restic-password ]] \
    && [[ -s /etc/aletheon/credentials/restic-repository ]]; then
    systemctl enable --now aletheon-backup.timer
  else
    systemctl disable --now aletheon-backup.timer
    systemctl reset-failed aletheon-backup.service || true
    echo "backup timer disabled: configure restic and non-empty protected credentials to enable it" >&2
  fi
fi

version_json=$(/usr/bin/aletheon version --json)
manifest_tmp=$(mktemp /var/lib/aletheon/state/.deployment-manifest.XXXXXX)
python3 - "$version_json" "$manifest_tmp" \
  "$previous_binary" "$previous_core_config" "$previous_user_config" \
  "$user_config" <<'PY'
import json
import sys

version = json.loads(sys.argv[1])
source_revision = version.get("source_revision", "")
if not source_revision or source_revision == "unknown":
    raise SystemExit("installed binary has no source revision")
binary_version = version.get("version", "")
if not binary_version:
    raise SystemExit("installed binary has no package version")
user_config = sys.argv[6]
if not user_config:
    raise SystemExit("system deployment requires the invoking user's config path")
manifest = {
    "installed_sha": source_revision,
    "core_runtime_version": binary_version,
    "user_runtime_version": binary_version,
    "previous_core_binary": sys.argv[3],
    "previous_user_binary": sys.argv[3],
    "previous_core_config": sys.argv[4],
    "previous_user_config": sys.argv[5],
    "core_binary": "/usr/bin/aletheon",
    "user_binary": "/usr/bin/aletheon",
    "core_config": "/etc/aletheon/config.toml",
    "user_config": user_config,
}
with open(sys.argv[2], "w", encoding="utf-8") as target:
    json.dump(manifest, target, sort_keys=True, indent=2)
    target.write("\n")
PY
install -o root -g aletheon -m 0640 "$manifest_tmp" \
  /var/lib/aletheon/state/deployment-manifest.json
rm -f -- "$manifest_tmp"
