#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
matrix="$repo_root/tests/production/failure_matrix.sh"

bash -n "$matrix"

legacy_socket='/run/aletheon/'"aletheon.sock"
! grep -Fq "$legacy_socket" "$matrix"
! grep -Eq '(^|[[:space:]])systemctl (kill|start|restart|stop).*aletheon\.service' "$matrix"
grep -Fq 'machine_unit=aletheon-core.service' "$matrix"
grep -Fq 'target_socket=$(installed_user_socket "$target_user")' "$matrix"
grep -Fq 'run_as_installed_user "$target_user" systemctl --user kill' "$matrix"
grep -Fq 'capture_sqlite_integrity "$machine_state"' "$matrix"
grep -Fq 'capture_sqlite_integrity "$target_user_state"' "$matrix"
grep -Fq 'validate_provenance "$before" provenance "$before_provenance"' "$matrix"
grep -Fq 'validate_provenance "$after" before_provenance "$before_provenance"' "$matrix"
grep -Fq 'validate_provenance "$after" provenance "$after_provenance"' "$matrix"
grep -Fq 'runtime_provenance:$provenance' "$matrix"

for field in boundary target_user target_uid user_unit user_socket user_pid \
  user_state_root machine_unit machine_socket machine_pid machine_state_root; do
  grep -Fq "$field" "$matrix" || {
    echo "failure matrix omits provenance field: $field" >&2
    exit 1
  }
done

echo "failure-matrix static multi-user topology verification: pass"
