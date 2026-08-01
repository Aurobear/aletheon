#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
binary=${ALETHEON_BINARY:-"$repo_root/target/release/aletheon"}
config_source=${ALETHEON_CONFIG:-"$repo_root/config/production.toml.example"}
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
install -d -o aletheon -g aletheon -m 0700 /var/cache/aletheon/backup
for secret in provider.env telegram.env gbrain.env; do
  if [[ ! -e /etc/aletheon/credentials/$secret ]]; then
    install -o aletheon -g aletheon -m 0600 /dev/null "/etc/aletheon/credentials/$secret"
  fi
done

install -o root -g root -m 0755 "$binary" /usr/bin/aletheon
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
