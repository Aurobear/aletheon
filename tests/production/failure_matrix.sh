#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
# shellcheck source=tests/production/lib/installed_host.sh
source "$repo_root/tests/production/lib/installed_host.sh"
require_disposable_installed_host
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
jq -e '.schema_version == 1
  and (.results.cross_domain_acceptance | startswith("verified_"))
  and (.results.functional_indicators | startswith("verified_"))
  and (.results.architecture | startswith("verified_"))' "$v01_report" >/dev/null || {
  echo "BLOCKED: V01 report does not prove PASS" >&2; exit 78;
}

for phase in event_append memory_lease gbrain_remote_success agent_runtime_completion; do
  before="$artifacts/$phase-before.json"
  after="$artifacts/$phase-after.json"
  "$driver" prepare "$phase" "$before"
  jq -e --arg phase "$phase" '.phase == $phase and .scope == "disposable" and .acknowledged_boundary == true and (.authoritative_state | type == "object")' "$before" >/dev/null
  systemctl kill --kill-who=main --signal=KILL aletheon.service
  systemctl reset-failed aletheon.service || true
  systemctl start aletheon.service
  "$repo_root/scripts/verify-systemd.sh" --readiness --socket /run/aletheon/aletheon.sock --timeout 30
  "$driver" verify "$phase" "$before" "$after"
  jq -e --arg phase "$phase" '.phase == $phase and .recovered == true and .idempotent == true and .silent_loss == false' "$after" >/dev/null
  capture_sqlite_integrity /var/lib/aletheon "$artifacts/$phase-integrity.txt"
done

for failure in queue_full disk_full corrupt_supplement provider_timeout tui_disconnect; do
  receipt="$artifacts/$failure.json"
  "$driver" inject "$failure" "$receipt"
  jq -e --arg failure "$failure" '.failure == $failure and .scope == "disposable" and .bounded == true and .degraded_visible == true and .silent_loss == false' "$receipt" >/dev/null
  "$driver" recover "$failure" "$receipt"
  jq -e '.recovered == true and .idempotent == true' "$receipt" >/dev/null
done

capture_sqlite_integrity /var/lib/aletheon "$artifacts/final-integrity.txt"
"$driver" compare-v01 "$v01_report" "$artifacts/v01-checksum-comparison.json"
jq -e '.projection_checksum_match == true and .state_checksum_match == true' \
  "$artifacts/v01-checksum-comparison.json" >/dev/null
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg artifacts "$artifacts" \
  '{status:"PASS",lane:"disposable-installed-host",completed_utc:$completed_utc,artifacts:$artifacts,ignored_cases:0}' \
  >"$artifacts/operator-receipt.json"
echo "failure matrix passed: $artifacts/operator-receipt.json"
