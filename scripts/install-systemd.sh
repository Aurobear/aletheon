#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
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
for secret in provider.env telegram.env gbrain.env; do
  if [[ ! -e /etc/aletheon/credentials/$secret ]]; then
    install -o aletheon -g aletheon -m 0600 /dev/null "/etc/aletheon/credentials/$secret"
  fi
done

install -o root -g root -m 0755 "$binary" /usr/bin/aletheon
install -D -o root -g root -m 0755 "$repo_root/scripts/verify-systemd.sh" \
  /usr/libexec/aletheon/verify-systemd.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/aletheon-secret-audit.sh" \
  /usr/libexec/aletheon/aletheon-secret-audit.sh
install -D -o root -g root -m 0755 "$repo_root/scripts/aletheon-secret-init.sh" \
  /usr/libexec/aletheon/aletheon-secret-init.sh
for helper in aletheon-healthcheck.sh backup-aletheon.sh restore-aletheon.sh \
  cleanup-aletheon.sh verify-network-exposure.sh upgrade-aletheon.sh; do
  install -D -o root -g root -m 0755 "$repo_root/scripts/$helper" \
    "/usr/libexec/aletheon/$helper"
done
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
systemd-analyze verify /etc/systemd/system/aletheon.service
systemd-analyze verify /etc/systemd/system/aletheon-core.service
systemd-analyze verify /etc/systemd/system/aletheon-backup.service
systemd-analyze verify /etc/systemd/system/aletheon-cleanup.service
systemd-analyze verify /usr/lib/systemd/user/aletheon.socket \
  /usr/lib/systemd/user/aletheon.service
systemctl daemon-reload
if ((enable)); then
  systemctl enable --now aletheon-core.service
  systemctl --global enable aletheon.socket
  systemctl enable --now aletheon-backup.timer aletheon-cleanup.timer
fi
