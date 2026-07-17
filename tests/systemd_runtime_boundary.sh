#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$repo_root"

forbidden_root='/home/'"aurobear/Bear-ws"
if grep -R "$forbidden_root" config/*.service config/*.socket; then
  echo 'deployment unit contains a developer-local path' >&2; exit 1
fi
grep -q '^ListenStream=%t/aletheon/aletheon.sock$' config/aletheon.user.socket
grep -q '^DirectoryMode=0700$' config/aletheon.user.socket
grep -q '^SocketMode=0600$' config/aletheon.user.socket
if grep -q '^RuntimeDirectory=' config/aletheon.user.service; then
  echo 'user service competes with socket activation for runtime-directory ownership' >&2; exit 1
fi
if grep -Eq 'ReadWritePaths=.*(/home|/tmp)' config/aletheon-core.service; then
  echo 'core unit grants a user or temporary writable root' >&2; exit 1
fi
grep -q 'ExecStart=.*aletheon core' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon core .*--socket /run/aletheon/core.sock' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon daemon' config/aletheon.user.service
grep -q '^EnvironmentFile=-/etc/aletheon/credentials/provider.env$' config/aletheon-core.service
if grep -q '^EnvironmentFile=-%d/provider.env$' config/aletheon-core.service; then
  echo 'core provider environment depends on a credential path unavailable during environment loading' >&2; exit 1
fi

# The static release lane validates the two distinct authority boundaries. The
# test script itself is a stable executable placeholder; unit verification does
# not execute the staged binary.
scripts/verify-systemd.sh --core-unit config/aletheon-core.service \
  --binary tests/systemd_runtime_boundary.sh
scripts/verify-systemd.sh --user-units \
  config/aletheon.user.service config/aletheon.user.socket \
  --binary tests/systemd_runtime_boundary.sh
negative_units=$(mktemp -d)
trap 'rm -rf "$negative_units"' EXIT
cp config/aletheon.user.service "$negative_units/aletheon.service"
sed 's/^SocketMode=0600$/SocketMode=0666/' config/aletheon.user.socket \
  >"$negative_units/aletheon.socket"
if scripts/verify-systemd.sh --user-units \
  "$negative_units/aletheon.service" "$negative_units/aletheon.socket" \
  --binary tests/systemd_runtime_boundary.sh >/dev/null 2>&1; then
  echo 'typed user verifier accepted a cross-user socket mode' >&2; exit 1
fi

# The legacy system unit is core-only, and installation enables the private
# socket rather than keeping every user's runtime resident.
grep -q 'ExecStart=.*aletheon core' config/aletheon.service
grep -q 'ExecStart=.*aletheon core .*--socket /run/aletheon/core.sock' config/aletheon.service
if grep -q 'ExecStart=.*aletheon daemon' config/aletheon.service; then
  echo 'compatibility system service still starts user execution' >&2; exit 1
fi
grep -q 'config/aletheon.user.socket' setup.sh
grep -q 'systemctl --user enable --now aletheon.socket' setup.sh
if grep -q 'systemctl --user enable aletheon.service' setup.sh; then
  echo 'setup still enables a permanently resident user service' >&2; exit 1
fi
grep -q 'config/aletheon-core.service' scripts/install-systemd.sh
grep -q 'systemctl disable --now aletheon.service' scripts/install-systemd.sh
grep -q 'systemctl restart aletheon-core.service' scripts/install-systemd.sh
grep -q 'systemctl --global enable aletheon.socket' scripts/install-systemd.sh
grep -q 'verify-systemd.sh --core-unit' scripts/install-systemd.sh
grep -q 'verify-systemd.sh --user-units' scripts/install-systemd.sh
if grep -q 'systemd-analyze --user verify' scripts/install-systemd.sh; then
  echo 'root installer depends on an unavailable user systemd manager' >&2; exit 1
fi
