#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$repo_root"

! grep -R '/home/aurobear/Bear-ws' config/*.service config/*.socket
grep -q '^ListenStream=%t/aletheon/aletheon.sock$' config/aletheon.user.socket
grep -q '^DirectoryMode=0700$' config/aletheon.user.socket
grep -q '^SocketMode=0600$' config/aletheon.user.socket
grep -q '^RuntimeDirectoryMode=0700$' config/aletheon.user.service
! grep -Eq 'ReadWritePaths=.*(/home|/tmp)' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon core' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon daemon' config/aletheon.user.service

# The legacy system unit is core-only, and installation enables the private
# socket rather than keeping every user's runtime resident.
grep -q 'ExecStart=.*aletheon core' config/aletheon.service
! grep -q 'ExecStart=.*aletheon daemon' config/aletheon.service
grep -q 'config/aletheon.user.socket' setup.sh
grep -q 'systemctl --user enable --now aletheon.socket' setup.sh
! grep -q 'systemctl --user enable aletheon.service' setup.sh
grep -q 'config/aletheon-core.service' scripts/install-systemd.sh
grep -q 'systemctl --global enable aletheon.socket' scripts/install-systemd.sh
