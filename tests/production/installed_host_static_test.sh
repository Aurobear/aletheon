#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
# shellcheck source=tests/production/lib/installed_host.sh
source "$repo_root/tests/production/lib/installed_host.sh"

set +e
(ALETHEON_DISPOSABLE_HOST=0 require_disposable_installed_host) >/dev/null 2>&1
status=$?
set -e
[[ $status -eq 78 ]] || {
  echo "disposable-host guard did not fail closed" >&2
  exit 1
}

unset ALETHEON_TEST_USER_A ALETHEON_TEST_USER_B
set +e
installed_test_users >/dev/null 2>&1
status=$?
set -e
[[ $status -eq 78 ]] || {
  echo "missing installed-host principals were accepted" >&2
  exit 1
}

ALETHEON_TEST_USER_A=alice ALETHEON_TEST_USER_B=alice
set +e
installed_test_users >/dev/null 2>&1
status=$?
set -e
[[ $status -eq 78 ]] || {
  echo "identical installed-host principals were accepted" >&2
  exit 1
}

id() {
  case "${1-}:${2-}" in
    -u:alice) printf '1001\n' ;;
    -u:bob) printf '1002\n' ;;
    *) command id "$@" ;;
  esac
}
ALETHEON_TEST_USER_A=alice ALETHEON_TEST_USER_B=bob
mapfile -t users < <(installed_test_users)
[[ ${users[*]} == 'alice bob' ]]
[[ $(installed_user_socket alice) == /run/user/1001/aletheon/aletheon.sock ]]
[[ $(installed_user_socket bob) == /run/user/1002/aletheon/aletheon.sock ]]

scoped_files=(
  "$repo_root/tests/production/install_upgrade_restart.sh"
  "$repo_root/tests/production/lib/installed_host.sh"
)
legacy_socket='/run/aletheon/'"aletheon.sock"
! grep -F "$legacy_socket" "${scoped_files[@]}"
grep -Fq 'aletheon-core.service' "${scoped_files[@]}"
grep -Fq '/run/user/%s/aletheon/aletheon.sock' "${scoped_files[@]}"
! grep -Fq -- '--readiness --socket /run/aletheon/core.sock' "${scoped_files[@]}"
# The installed upgrade drill must exercise the shipped multi-user defaults;
# wrappers are not permitted to translate stale system-service assumptions.
! grep -Fq 'ALETHEON_SYSTEMCTL_COMMAND=' "$repo_root/tests/production/install_upgrade_restart.sh"
! grep -Fq 'ALETHEON_HEALTHCHECK_COMMAND=' "$repo_root/tests/production/install_upgrade_restart.sh"
grep -Fq 'core_unit=${ALETHEON_CORE_UNIT:-aletheon-core.service}' "$repo_root/scripts/upgrade-aletheon.sh"
grep -Fq -- '--authorized-users' "$repo_root/tests/production/install_upgrade_restart.sh"
grep -Fq -- '--user-backup-command' "$repo_root/tests/production/install_upgrade_restart.sh"

echo "installed-host static topology verification: pass"
