#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
# shellcheck source=tests/production/lib/installed_host.sh
source "$repo_root/tests/production/lib/installed_host.sh"
require_disposable_installed_host
installed_test_users >/dev/null

artifacts=$(init_release_artifacts)/failure-matrix
install -d -m 0700 "$artifacts"
driver=${ALETHEON_PRODUCTION_FAILURE_DRIVER:-}
[[ -n "$driver" && -x "$driver" && ! -L "$driver" ]] || {
  echo "BLOCKED: ALETHEON_PRODUCTION_FAILURE_DRIVER must drive real daemon boundaries" >&2; exit 78;
}
v01_report=${ALETHEON_V01_ACCEPTANCE_REPORT:-}
[[ -f "$v01_report" && ! -L "$v01_report" ]] || {
  echo "BLOCKED: V01 machine-readable acceptance report is required" >&2; exit 78;
}
v01_recipe_receipt=${ALETHEON_V01_RECIPE_RECEIPT:-}
[[ -f "$v01_recipe_receipt" && ! -L "$v01_recipe_receipt" ]] || {
  echo "BLOCKED: failure matrix requires the aggregate gate's V01 recipe receipt" >&2; exit 78;
}
report_sha256=$(sha256sum "$v01_report" | cut -d' ' -f1)
jq -e --arg report_sha256 "$report_sha256" \
  '.schema_version == 1 and .status == "passed_in_aggregate_release_gate"
   and .command == "just acceptance" and .report_sha256 == $report_sha256' \
  "$v01_recipe_receipt" >/dev/null || {
  echo "BLOCKED: V01 recipe receipt does not match the acceptance report" >&2; exit 78;
}
"$repo_root/scripts/release-acceptance.sh" --validate-v01-report "$v01_report"

target_user=${ALETHEON_FAILURE_TEST_USER:-${ALETHEON_TEST_USER_A:-}}
mapfile -t installed_users < <(installed_test_users)
printf '%s\n' "${installed_users[@]}" | grep -Fxq -- "$target_user" || {
  echo "BLOCKED: ALETHEON_FAILURE_TEST_USER must name one installed test user" >&2; exit 78;
}
target_uid=$(installed_user_uid "$target_user")
target_socket=$(installed_user_socket "$target_user")
target_user_state=$(installed_user_state_root "$target_user")
peer_user=
for user in "${installed_users[@]}"; do
  [[ "$user" == "$target_user" ]] || { peer_user=$user; break; }
done
[[ -n "$peer_user" ]] || { echo "BLOCKED: failure matrix requires a peer test user" >&2; exit 78; }
peer_uid=$(installed_user_uid "$peer_user")
peer_user_state=$(installed_user_state_root "$peer_user")
machine_unit=aletheon-core.service
machine_socket=/run/aletheon/core.sock
machine_state=/var/lib/aletheon
user_unit=aletheon.service
candidate=${ALETHEON_RELEASE_BINARY:-}
[[ -n "$candidate" && -x "$candidate" && ! -L "$candidate" ]] || {
  echo "BLOCKED: ALETHEON_RELEASE_BINARY must identify the candidate under test" >&2; exit 78;
}
candidate_sha256=$(sha256sum "$candidate" | cut -d' ' -f1)
driver_timeout=${ALETHEON_FAILURE_DRIVER_TIMEOUT_SECS:-120}
[[ "$driver_timeout" =~ ^[1-9][0-9]*$ && "$driver_timeout" -le 600 ]] || {
  echo "BLOCKED: ALETHEON_FAILURE_DRIVER_TIMEOUT_SECS must be between 1 and 600" >&2; exit 78;
}
command -v timeout >/dev/null || { echo "BLOCKED: timeout is required" >&2; exit 78; }

runtime_provenance() {
  local machine_pid user_pid machine_binary user_binary machine_binary_sha256 user_binary_sha256
  machine_pid=$(systemctl show "$machine_unit" -p MainPID --value)
  user_pid=$(run_as_installed_user "$target_user" \
    systemctl --user show "$user_unit" -p MainPID --value)
  [[ "$machine_pid" =~ ^[1-9][0-9]*$ && "$user_pid" =~ ^[1-9][0-9]*$ ]] || {
    echo "installed multi-user runtime has no live core/user PID" >&2
    return 1
  }
  machine_binary=$(readlink -f "/proc/$machine_pid/exe")
  user_binary=$(readlink -f "/proc/$user_pid/exe")
  machine_binary_sha256=$(sha256sum "$machine_binary" | cut -d' ' -f1)
  user_binary_sha256=$(sha256sum "$user_binary" | cut -d' ' -f1)
  [[ "$machine_binary_sha256" == "$candidate_sha256" && "$user_binary_sha256" == "$candidate_sha256" ]] || {
    echo "installed runtime is not executing ALETHEON_RELEASE_BINARY" >&2
    return 1
  }
  jq -cn \
    --arg boundary per-user-runtime \
    --arg target_user "$target_user" --argjson target_uid "$target_uid" \
    --arg user_unit "$user_unit" --arg user_socket "$target_socket" \
    --argjson user_pid "$user_pid" --arg user_state_root "$target_user_state" \
    --arg machine_unit "$machine_unit" --arg machine_socket "$machine_socket" \
    --argjson machine_pid "$machine_pid" --arg machine_state_root "$machine_state" \
    --arg candidate_sha256 "$candidate_sha256" \
    --arg peer_user "$peer_user" --argjson peer_uid "$peer_uid" --arg peer_state_root "$peer_user_state" \
    '{boundary:$boundary,target_user:$target_user,target_uid:$target_uid,
      candidate_sha256:$candidate_sha256,
      user:{unit:$user_unit,socket:$user_socket,pid:$user_pid,state_root:$user_state_root},
      peer:{user:$peer_user,uid:$peer_uid,state_root:$peer_state_root},
      machine:{unit:$machine_unit,socket:$machine_socket,pid:$machine_pid,state_root:$machine_state_root}}'
}

publish_driver_provenance() {
  export ALETHEON_FAILURE_TARGET_USER="$target_user"
  export ALETHEON_FAILURE_TARGET_UID="$target_uid"
  export ALETHEON_FAILURE_USER_UNIT="$user_unit"
  export ALETHEON_FAILURE_USER_SOCKET="$target_socket"
  export ALETHEON_FAILURE_USER_STATE_ROOT="$target_user_state"
  export ALETHEON_FAILURE_MACHINE_UNIT="$machine_unit"
  export ALETHEON_FAILURE_MACHINE_SOCKET="$machine_socket"
  export ALETHEON_FAILURE_MACHINE_STATE_ROOT="$machine_state"
  export ALETHEON_FAILURE_PEER_USER="$peer_user"
  export ALETHEON_FAILURE_PEER_UID="$peer_uid"
  export ALETHEON_FAILURE_PEER_STATE_ROOT="$peer_user_state"
  export ALETHEON_FAILURE_CANDIDATE_SHA256="$candidate_sha256"
  export ALETHEON_FAILURE_PROVENANCE_JSON=$1
}

run_driver() {
  timeout --signal=TERM --kill-after=5s "${driver_timeout}s" "$driver" "$@"
}

state_root_hash() {
  local root=$1
  [[ -d "$root" && ! -L "$root" ]] || { echo "state root is unavailable: $root" >&2; return 1; }
  local manifest
  manifest=$(mktemp)
  find "$root" -xdev -type f -printf '%P\0' | sort -z | while IFS= read -r -d '' relative; do
    printf '%s  %s\n' "$(sha256sum "$root/$relative" | cut -d' ' -f1)" "$relative"
  done >"$manifest"
  [[ -s "$manifest" ]] || { rm -f "$manifest"; echo "state root has no files: $root" >&2; return 1; }
  sha256sum "$manifest" | cut -d' ' -f1
  rm -f "$manifest"
}

runtime_state_hashes() {
  jq -cn --arg machine "$(state_root_hash "$machine_state")" \
    --arg target_user "$(state_root_hash "$target_user_state")" \
    --arg peer_user "$(state_root_hash "$peer_user_state")" \
    '{machine:$machine,target_user:$target_user,peer_user:$peer_user}'
}

validate_common_receipt() {
  local receipt=$1 provenance=$2 state_hashes=$3
  validate_provenance "$receipt" provenance "$provenance"
  jq -e --argjson state_hashes "$state_hashes" --arg candidate "$candidate_sha256" \
    '.candidate_sha256 == $candidate and .state_hashes == $state_hashes
     and .cross_scope_leak == false and (.ignored_cases | type == "array" and length == 0)' \
    "$receipt" >/dev/null
}

validate_provenance() {
  local receipt=$1 field=$2 expected=$3
  jq -e --argjson expected "$expected" --arg field "$field" \
    '.[$field] == $expected' "$receipt" >/dev/null || {
    echo "failure driver receipt is not bound to $field runtime provenance: $receipt" >&2
    return 1
  }
}

capture_runtime_integrity() {
  local prefix=$1
  capture_sqlite_integrity "$machine_state" "$prefix-machine-integrity.txt"
  capture_sqlite_integrity "$target_user_state" "$prefix-user-$target_uid-integrity.txt"
}

# Keep the machine inference core alive while crashing only the selected user's
# private runtime. This exercises the boundary that owns sessions, memory,
# integrations and Agent execution without falling back to the compatibility unit.
systemctl is-active --quiet "$machine_unit"
run_as_installed_user "$target_user" systemctl --user start "$user_unit"
"$repo_root/scripts/verify-systemd.sh" --readiness --socket "$target_socket" --timeout 30

for phase in event_append memory_lease gbrain_remote_success agent_runtime_completion; do
  before="$artifacts/$phase-before.json"
  after="$artifacts/$phase-after.json"
  before_provenance=$(runtime_provenance)
  before_machine_pid=$(jq -r '.machine.pid' <<<"$before_provenance")
  before_user_pid=$(jq -r '.user.pid' <<<"$before_provenance")
  publish_driver_provenance "$before_provenance"
  run_driver prepare "$phase" "$before"
  before_state_hashes=$(runtime_state_hashes)
  jq -e --arg phase "$phase" \
    '.phase == $phase and .scope == "disposable" and .acknowledged_boundary == true
     and (.authoritative_state | type == "object")
     and (.acknowledged_work.id | type == "string" and length > 0)
     and .acknowledged_work.kind == $phase and .acknowledged_work.state == "acknowledged"' \
    "$before" >/dev/null
  validate_common_receipt "$before" "$before_provenance" "$before_state_hashes"
  acknowledged_work_id=$(jq -r '.acknowledged_work.id' "$before")

  run_as_installed_user "$target_user" systemctl --user kill \
    --kill-who=main --signal=KILL "$user_unit"
  run_as_installed_user "$target_user" systemctl --user reset-failed "$user_unit" || true
  run_as_installed_user "$target_user" systemctl --user start "$user_unit"
  "$repo_root/scripts/verify-systemd.sh" --readiness --socket "$target_socket" --timeout 30

  after_provenance=$(runtime_provenance)
  [[ $(jq -r '.machine.pid' <<<"$after_provenance") == "$before_machine_pid" ]] || {
    echo "user-runtime failure unexpectedly restarted the machine core" >&2; exit 1;
  }
  [[ $(jq -r '.user.pid' <<<"$after_provenance") != "$before_user_pid" ]] || {
    echo "selected user runtime PID did not change after SIGKILL" >&2; exit 1;
  }
  publish_driver_provenance "$after_provenance"
  run_driver verify "$phase" "$before" "$after"
  after_state_hashes=$(runtime_state_hashes)
  [[ $(jq -r '.peer_user' <<<"$after_state_hashes") == \
    "$(jq -r '.peer_user' <<<"$before_state_hashes")" ]] || {
    echo "failure recovery changed the peer user's state" >&2; exit 1;
  }
  jq -e --arg phase "$phase" --arg work_id "$acknowledged_work_id" \
    --argjson before_state_hashes "$before_state_hashes" \
    '.phase == $phase and .recovered == true and .idempotent == true and .silent_loss == false
     and .acknowledged_work.id == $work_id and .acknowledged_work.state == "settled"
     and .before_state_hashes == $before_state_hashes' \
    "$after" >/dev/null
  validate_provenance "$after" before_provenance "$before_provenance"
  validate_common_receipt "$after" "$after_provenance" "$after_state_hashes"
  capture_runtime_integrity "$artifacts/$phase"
done

for failure in queue_full disk_full corrupt_supplement provider_timeout tui_disconnect; do
  receipt="$artifacts/$failure.json"
  failure_provenance=$(runtime_provenance)
  peer_hash_before=$(state_root_hash "$peer_user_state")
  publish_driver_provenance "$failure_provenance"
  run_driver inject "$failure" "$receipt"
  failure_state_hashes=$(runtime_state_hashes)
  jq -e --arg failure "$failure" \
    '.failure == $failure and .scope == "disposable" and .bounded == true
     and .degraded_visible == true and .silent_loss == false
     and (.acknowledged_work.id | type == "string" and length > 0)
     and .acknowledged_work.kind == $failure and .acknowledged_work.state == "observed"' \
    "$receipt" >/dev/null
  validate_common_receipt "$receipt" "$failure_provenance" "$failure_state_hashes"
  injected_work_id=$(jq -r '.acknowledged_work.id' "$receipt")
  run_driver recover "$failure" "$receipt"
  recovered_provenance=$(runtime_provenance)
  recovered_state_hashes=$(runtime_state_hashes)
  [[ $(state_root_hash "$peer_user_state") == "$peer_hash_before" ]] || {
    echo "injected failure changed the peer user's state" >&2; exit 1;
  }
  jq -e --arg work_id "$injected_work_id" --argjson state_hashes "$recovered_state_hashes" \
    --arg candidate "$candidate_sha256" \
    '.recovered == true and .idempotent == true and .cross_scope_leak == false
     and .candidate_sha256 == $candidate and .recovery_state_hashes == $state_hashes
     and .acknowledged_work.id == $work_id and .acknowledged_work.state == "settled"
     and (.ignored_cases | type == "array" and length == 0)' "$receipt" >/dev/null
  validate_provenance "$receipt" recovery_provenance "$recovered_provenance"
  capture_runtime_integrity "$artifacts/$failure"
done

backup_root="$artifacts/matching-backup"
backup_receipt="$artifacts/matching-backup.json"
pre_backup_provenance=$(runtime_provenance)
publish_driver_provenance "$pre_backup_provenance"
run_driver backup-matching "$backup_root" "$backup_receipt"
pre_backup_hashes=$(runtime_state_hashes)
validate_common_receipt "$backup_receipt" "$pre_backup_provenance" "$pre_backup_hashes"
backup_id=$(jq -r '.backup_id // empty' "$backup_receipt")
[[ -n "$backup_id" && -d "$backup_root" && ! -L "$backup_root" ]] || {
  echo "matching backup driver did not produce a safe backup and identity" >&2; exit 1;
}
jq -e '.status == "complete" and .matching_binary_and_state == true' "$backup_receipt" >/dev/null

restore_receipt="$artifacts/matching-restore.json"
run_driver restore-matching "$backup_receipt" "$restore_receipt"
restored_provenance=$(runtime_provenance)
restored_state_hashes=$(runtime_state_hashes)
[[ "$restored_state_hashes" == "$pre_backup_hashes" ]] || {
  echo "matching restore did not reproduce the backed-up state hashes" >&2; exit 1;
}
jq -e --arg backup_id "$backup_id" --arg candidate "$candidate_sha256" \
  --argjson state_hashes "$restored_state_hashes" \
  '.status == "restored" and .backup_id == $backup_id and .matching_binary_and_state == true
   and .candidate_sha256 == $candidate and .state_hashes == $state_hashes
   and .cross_scope_leak == false and (.ignored_cases | type == "array" and length == 0)' \
  "$restore_receipt" >/dev/null
validate_provenance "$restore_receipt" provenance "$restored_provenance"

final_provenance=$restored_provenance
publish_driver_provenance "$final_provenance"
capture_runtime_integrity "$artifacts/final"
run_driver compare-v01 "$restore_receipt" "$v01_report" \
  "$artifacts/v01-checksum-comparison.json"
jq -e --arg candidate "$candidate_sha256" --argjson state_hashes "$restored_state_hashes" \
  '.projection_checksum_match == true and .state_checksum_match == true
   and .candidate_sha256 == $candidate and .state_hashes == $state_hashes
   and .cross_scope_leak == false and (.ignored_cases | type == "array" and length == 0)' \
  "$artifacts/v01-checksum-comparison.json" >/dev/null
validate_provenance "$artifacts/v01-checksum-comparison.json" provenance "$final_provenance"
all_receipts=("$artifacts"/*-before.json "$artifacts"/*-after.json "$artifacts"/{queue_full,disk_full,corrupt_supplement,provider_timeout,tui_disconnect}.json "$backup_receipt" "$restore_receipt" "$artifacts/v01-checksum-comparison.json")
ignored_inventory=$(jq -s '[.[].ignored_cases[]?]' "${all_receipts[@]}")
[[ $(jq 'length' <<<"$ignored_inventory") -eq 0 ]] || {
  echo "failure matrix contains ignored cases" >&2; exit 1;
}
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg artifacts "$artifacts" \
  --arg driver_sha256 "$(sha256sum "$driver" | cut -d' ' -f1)" \
  --arg candidate_sha256 "$candidate_sha256" --arg backup_id "$backup_id" \
  --argjson provenance "$final_provenance" --argjson ignored_inventory "$ignored_inventory" \
  '{status:"PASS",lane:"disposable-installed-host",completed_utc:$completed_utc,artifacts:$artifacts,
    external_failure_driver:"required_real_driver",driver_sha256:$driver_sha256,
    candidate_sha256:$candidate_sha256,matching_backup_id:$backup_id,
    runtime_provenance:$provenance,ignored_inventory:$ignored_inventory,
    ignored_cases:($ignored_inventory|length)}' \
  >"$artifacts/operator-receipt.json"
echo "failure matrix passed: $artifacts/operator-receipt.json"
