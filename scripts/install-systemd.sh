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
for secret in provider.env telegram.env google-vault.key gbrain.env; do
  if [[ ! -e /etc/aletheon/credentials/$secret ]]; then
    install -o aletheon -g aletheon -m 0600 /dev/null "/etc/aletheon/credentials/$secret"
  fi
done

install -o root -g root -m 0755 "$binary" /usr/bin/aletheon
install -D -o root -g root -m 0755 "$repo_root/scripts/verify-systemd.sh" \
  /usr/libexec/aletheon/verify-systemd.sh
install -o root -g root -m 0644 "$repo_root/config/aletheon.service" \
  /etc/systemd/system/aletheon.service
if [[ ! -e /etc/aletheon/config.toml ]]; then
  install -o root -g aletheon -m 0640 "$config_source" /etc/aletheon/config.toml
fi
install -D -o root -g root -m 0644 "$repo_root/docs/deployment/systemd.md" \
  /usr/share/doc/aletheon/systemd.md

/usr/libexec/aletheon/verify-systemd.sh --preflight \
  --binary /usr/bin/aletheon --config /etc/aletheon/config.toml
systemd-analyze verify /etc/systemd/system/aletheon.service
systemctl daemon-reload
if ((enable)); then
  systemctl enable --now aletheon.service
fi
