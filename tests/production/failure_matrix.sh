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
target_home=$(installed_user_home "$target_user")
target_socket=$(installed_user_socket "$target_user")
target_user_state="$target_home/.local/share/aletheon"
machine_unit=aletheon-core.service
machine_socket=/run/aletheon/core.sock
machine_state=/var/lib/aletheon
user_unit=aletheon.service

runtime_provenance() {
  local machine_pid user_pid
  machine_pid=$(systemctl show "$machine_unit" -p MainPID --value)
  user_pid=$(run_as_installed_user "$target_user" \
    systemctl --user show "$user_unit" -p MainPID --value)
  [[ "$machine_pid" =~ ^[1-9][0-9]*$ && "$user_pid" =~ ^[1-9][0-9]*$ ]] || {
    echo "installed multi-user runtime has no live core/user PID" >&2
    return 1
  }
  jq -cn \
    --arg boundary per-user-runtime \
    --arg target_user "$target_user" --argjson target_uid "$target_uid" \
    --arg user_unit "$user_unit" --arg user_socket "$target_socket" \
    --argjson user_pid "$user_pid" --arg user_state_root "$target_user_state" \
    --arg machine_unit "$machine_unit" --arg machine_socket "$machine_socket" \
    --argjson machine_pid "$machine_pid" --arg machine_state_root "$machine_state" \
    '{boundary:$boundary,target_user:$target_user,target_uid:$target_uid,
      user:{unit:$user_unit,socket:$user_socket,pid:$user_pid,state_root:$user_state_root},
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
  export ALETHEON_FAILURE_PROVENANCE_JSON=$1
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
  "$driver" prepare "$phase" "$before"
  jq -e --arg phase "$phase" \
    '.phase == $phase and .scope == "disposable" and .acknowledged_boundary == true
     and (.authoritative_state | type == "object")' "$before" >/dev/null
  validate_provenance "$before" provenance "$before_provenance"

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
  "$driver" verify "$phase" "$before" "$after"
  jq -e --arg phase "$phase" \
    '.phase == $phase and .recovered == true and .idempotent == true and .silent_loss == false' \
    "$after" >/dev/null
  validate_provenance "$after" before_provenance "$before_provenance"
  validate_provenance "$after" provenance "$after_provenance"
  capture_runtime_integrity "$artifacts/$phase"
done

for failure in queue_full disk_full corrupt_supplement provider_timeout tui_disconnect; do
  receipt="$artifacts/$failure.json"
  failure_provenance=$(runtime_provenance)
  publish_driver_provenance "$failure_provenance"
  "$driver" inject "$failure" "$receipt"
  jq -e --arg failure "$failure" \
    '.failure == $failure and .scope == "disposable" and .bounded == true
     and .degraded_visible == true and .silent_loss == false' "$receipt" >/dev/null
  validate_provenance "$receipt" provenance "$failure_provenance"
  "$driver" recover "$failure" "$receipt"
  recovered_provenance=$(runtime_provenance)
  jq -e '.recovered == true and .idempotent == true' "$receipt" >/dev/null
  validate_provenance "$receipt" recovery_provenance "$recovered_provenance"
  capture_runtime_integrity "$artifacts/$failure"
done

final_provenance=$(runtime_provenance)
publish_driver_provenance "$final_provenance"
capture_runtime_integrity "$artifacts/final"
"$driver" compare-v01 "$v01_report" "$artifacts/v01-checksum-comparison.json"
jq -e '.projection_checksum_match == true and .state_checksum_match == true' \
  "$artifacts/v01-checksum-comparison.json" >/dev/null
validate_provenance "$artifacts/v01-checksum-comparison.json" provenance "$final_provenance"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg artifacts "$artifacts" \
  --arg driver_sha256 "$(sha256sum "$driver" | cut -d' ' -f1)" \
  --argjson provenance "$final_provenance" \
  '{status:"PASS",lane:"disposable-installed-host",completed_utc:$completed_utc,artifacts:$artifacts,
    external_failure_driver:"required_real_driver",driver_sha256:$driver_sha256,
    runtime_provenance:$provenance,ignored_cases:0}' \
  >"$artifacts/operator-receipt.json"
echo "failure matrix passed: $artifacts/operator-receipt.json"
