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
grep -Fq 'target_user_state=$(installed_user_state_root "$target_user")' "$matrix"
grep -Fq 'peer_user_state=$(installed_user_state_root "$peer_user")' "$matrix"
grep -Fq 'run_as_installed_user "$target_user" systemctl --user kill' "$matrix"
grep -Fq 'capture_sqlite_integrity "$machine_state"' "$matrix"
grep -Fq 'capture_sqlite_integrity "$target_user_state"' "$matrix"
grep -Fq 'validate_common_receipt "$before" "$before_provenance" "$before_state_hashes"' "$matrix"
grep -Fq 'validate_provenance "$after" before_provenance "$before_provenance"' "$matrix"
grep -Fq 'validate_common_receipt "$after" "$after_provenance" "$after_state_hashes"' "$matrix"
grep -Fq 'runtime_provenance:$provenance' "$matrix"
grep -Fq 'timeout --signal=TERM --kill-after=5s' "$matrix"
! grep -Eq '^ *"\$driver" +(prepare|verify|inject|recover|backup-matching|restore-matching|compare-v01)' "$matrix"
grep -Fq 'run_driver backup-matching "$backup_root" "$backup_receipt"' "$matrix"
grep -Fq 'run_driver restore-matching "$backup_receipt" "$restore_receipt"' "$matrix"
grep -Fq 'run_driver compare-v01 "$restore_receipt" "$v01_report"' "$matrix"
grep -Fq '.acknowledged_work.id' "$matrix"
grep -Fq '.cross_scope_leak == false' "$matrix"
grep -Fq '.state_hashes == $state_hashes' "$matrix"
grep -Fq 'ignored_inventory=$(jq -s' "$matrix"
grep -Fq 'candidate_sha256:$candidate_sha256' "$matrix"

for field in boundary target_user target_uid user_unit user_socket user_pid \
  user_state_root peer_user peer_uid peer_state_root candidate_sha256 \
  machine_unit machine_socket machine_pid machine_state_root; do
  grep -Fq "$field" "$matrix" || {
    echo "failure matrix omits provenance field: $field" >&2
    exit 1
  }
done

echo "failure-matrix static multi-user topology verification: pass"
