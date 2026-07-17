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
grep -q '^RuntimeDirectoryMode=0700$' config/aletheon.user.service
if grep -Eq 'ReadWritePaths=.*(/home|/tmp)' config/aletheon-core.service; then
  echo 'core unit grants a user or temporary writable root' >&2; exit 1
fi
grep -q 'ExecStart=.*aletheon core' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon daemon' config/aletheon.user.service

# The legacy system unit is core-only, and installation enables the private
# socket rather than keeping every user's runtime resident.
grep -q 'ExecStart=.*aletheon core' config/aletheon.service
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
grep -q 'systemctl --global enable aletheon.socket' scripts/install-systemd.sh
if grep -q 'systemd-analyze --user verify' scripts/install-systemd.sh; then
  echo 'root installer depends on an unavailable user systemd manager' >&2; exit 1
fi
