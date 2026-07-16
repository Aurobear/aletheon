#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
validate_v01_report() {
  local report_path=$1
  [[ -f "$report_path" && ! -L "$report_path" ]] || {
    echo "BLOCKED: V01 report missing or unsafe: $report_path" >&2; return 78;
  }
  python3 - "$report_path" <<'PY'
import json, pathlib, re, sys
report=json.loads(pathlib.Path(sys.argv[1]).read_text())
def fail(message): raise SystemExit(f"V01 acceptance: {message}")
checksum=re.compile(r"[0-9a-f]{64}").fullmatch
if report.get("schema_version") != 1 or report.get("fixture_version") != 1:
    fail("schema or fixture version is not 1")
for field in ("fixture_sha256", "config_schema_sha256"):
    if not isinstance(report.get(field), str) or not checksum(report[field]):
        fail(f"invalid {field}")
results=report.get("results")
if not isinstance(results, dict): fail("results is not an object")
runtime=results.get("cross_domain_acceptance")
if not isinstance(runtime, dict): fail("cross-domain result is missing")
if not isinstance(runtime.get("event_checksum"), str) or not checksum(runtime["event_checksum"]):
    fail("invalid event checksum")
projections=runtime.get("projection_checksums")
expected_projections={"agent_tree", "debug", "memory_jobs", "metrics", "session"}
if not isinstance(projections, dict) or set(projections) != expected_projections:
    fail("projection inventory drifted")
if any(not isinstance(value, str) or not checksum(value) for value in projections.values()):
    fail("invalid projection checksum")
for field in ("agent_runs_reopened", "mailbox_deliveries_reopened"):
    if not isinstance(runtime.get(field), int) or isinstance(runtime[field], bool) or runtime[field] <= 0:
        fail(f"{field} must be greater than zero")
if runtime.get("memory_lease_recovered") is not True: fail("memory lease was not recovered")
if runtime.get("unexpected_external_calls") != 0: fail("unexpected external calls were observed")
definitions=report.get("indicator_definitions")
indicators=results.get("functional_indicators")
if (not isinstance(definitions, list) or not definitions
        or any(not isinstance(name, str) or not name for name in definitions)
        or len(definitions) != len(set(definitions))):
    fail("indicator definition inventory is invalid")
if not isinstance(indicators, list) or len(indicators) != len(definitions):
    fail("functional indicator inventory drifted")
if any(not isinstance(item, dict) or not isinstance(item.get("name"), str)
       or item.get("passed") is not True for item in indicators):
    fail("one or more functional indicators failed")
if {item.get("name") for item in indicators} != set(definitions):
    fail("functional indicator names do not match definitions")
ablations=results.get("ablations")
if not isinstance(ablations, dict) or set(ablations) != {"workspace", "recurrence", "dasein"}:
    fail("ablation inventory drifted")
for name, measurement in ablations.items():
    if not isinstance(measurement, dict): fail(f"ablation {name} is malformed")
    baseline, ablated=measurement.get("baseline"), measurement.get("ablated")
    if (not isinstance(baseline, (int, float)) or isinstance(baseline, bool)
            or not isinstance(ablated, (int, float)) or isinstance(ablated, bool)
            or baseline <= ablated):
        fail(f"ablation {name} did not reduce its target metric")
architecture=results.get("architecture_gate")
if not isinstance(architecture, dict) or architecture.get("command") != "just architecture-check":
    fail("architecture gate command drifted")
if architecture.get("status") != "required_by_acceptance_recipe_not_inferred_from_test_receipts":
    fail("architecture gate marker drifted")
print("V01 acceptance report verified")
PY
}
if [[ ${1:-} == --validate-v01-report ]]; then
  [[ $# -eq 2 ]] || { echo "usage: $0 --validate-v01-report FILE" >&2; exit 64; }
  validate_v01_report "$2"
  exit
fi
artifacts=${ALETHEON_RELEASE_ACCEPTANCE_ARTIFACTS:-"$repo_root/target/release-acceptance"}
[[ "$artifacts" == "$repo_root"/target/* ]] || {
  echo "release acceptance artifacts must be below target/" >&2; exit 64;
}
if [[ -e "$artifacts" ]] && find "$artifacts" -mindepth 1 -print -quit | grep -q .; then
  echo "release acceptance requires a clean artifact directory: $artifacts" >&2; exit 1
fi
install -d -m 0700 "$artifacts"
exec > >(tee "$artifacts/release-acceptance.log") 2>&1

# Installed-host lanes run as root inside the disposable guest and may move the
# guest's /var/lib/aletheon during rollback. Keep their writable evidence away
# from the source checkout, then collect it into the clean repository bundle on
# every exit, including a blocked or failed drill.
if [[ -n ${ALETHEON_GUEST_RELEASE_ARTIFACTS:-} ]]; then
  guest_artifacts=$ALETHEON_GUEST_RELEASE_ARTIFACTS
  [[ "$guest_artifacts" == /var/tmp/* || "$guest_artifacts" == /tmp/* ]] || {
    echo "guest release artifacts must be below /var/tmp or /tmp" >&2; exit 64;
  }
  if [[ -e "$guest_artifacts" ]] && find "$guest_artifacts" -mindepth 1 -print -quit | grep -q .; then
    echo "guest release acceptance requires a clean artifact directory: $guest_artifacts" >&2; exit 1
  fi
  install -d -m 0700 "$guest_artifacts"
else
  guest_artifacts=$(mktemp -d /var/tmp/aletheon-release-acceptance.XXXXXX)
  chmod 0700 "$guest_artifacts"
fi
collect_guest_artifacts() {
  local status=$1 copy_status=0
  trap - EXIT
  set +e
  install -d -m 0700 "$artifacts/guest"
  cp -a -- "$guest_artifacts/." "$artifacts/guest/" || copy_status=$?
  printf '%s\n' "$guest_artifacts" >"$artifacts/guest-source-path.txt" || copy_status=$?
  if ((status == 0 && copy_status != 0)); then status=$copy_status; fi
  exit "$status"
}
trap 'collect_guest_artifacts $?' EXIT

command -v just >/dev/null || { echo "BLOCKED: just is required so the V01 acceptance recipe cannot be bypassed" >&2; exit 78; }
just --justfile "$repo_root/justfile" acceptance
v01_report=${ALETHEON_V01_ACCEPTANCE_REPORT:-"$repo_root/target/acceptance/acceptance.json"}
validate_v01_report "$v01_report"
v01_recipe_receipt="$guest_artifacts/v01-acceptance-recipe.json"
python3 - "$v01_report" "$v01_recipe_receipt" <<'PY'
import datetime, hashlib, json, pathlib, sys
report=pathlib.Path(sys.argv[1])
pathlib.Path(sys.argv[2]).write_text(json.dumps({
    "schema_version": 1,
    "status": "passed_in_aggregate_release_gate",
    "command": "just acceptance",
    "report_sha256": hashlib.sha256(report.read_bytes()).hexdigest(),
    "completed_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}, sort_keys=True) + "\n")
PY

"$repo_root/scripts/verify-migration-matrix.sh"
ALETHEON_RELEASE_ARTIFACTS="$guest_artifacts/installed-host" \
  "$repo_root/tests/production/install_upgrade_restart.sh"
(
  cd "$repo_root/tools/aletheon-monitor"
  python3 -m pytest -q tests
  python3 -m src.__main__ scenario --suite production --source-root "$repo_root" \
    | tee "$artifacts/production-scenarios.json"
)
ALETHEON_RELEASE_ARTIFACTS="$guest_artifacts" ALETHEON_V01_ACCEPTANCE_REPORT="$v01_report" \
  ALETHEON_V01_RECIPE_RECEIPT="$v01_recipe_receipt" \
  "$repo_root/tests/production/failure_matrix.sh"
"$repo_root/scripts/architecture-check.sh"
cargo tree --workspace --edges normal >"$artifacts/dependency-tree.txt"

python3 - "$artifacts/production-scenarios.json" <<'PY'
import json, pathlib, sys
report=json.loads(pathlib.Path(sys.argv[1]).read_text())
if report.get("status") != "PASS": raise SystemExit("production scenario report did not pass")
if report.get("summary", {}).get("BLOCKED", 0): raise SystemExit("production scenarios contain blocked cases")
PY
operator=${ALETHEON_RELEASE_OPERATOR:-}
[[ -n "$operator" ]] || { echo "BLOCKED: ALETHEON_RELEASE_OPERATOR is required for the release receipt" >&2; exit 78; }
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg operator "$operator" \
  --arg v01_report "$v01_report" --arg artifacts "$artifacts" \
  --arg guest_artifacts "$guest_artifacts" \
  '{status:"PASS",completed_utc:$completed_utc,operator:$operator,v01_report:$v01_report,artifacts:$artifacts,guest_artifacts:$guest_artifacts,guest_bundle:"guest",external_failure_driver:"required_real_host_driver_receipted",failure_driver_receipt:"guest/failure-matrix/operator-receipt.json",ignored_release_cases:0}' \
  >"$artifacts/operator-receipt.json"
echo "release acceptance passed: $artifacts/operator-receipt.json"
