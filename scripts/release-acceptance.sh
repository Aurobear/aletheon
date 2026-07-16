#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
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
[[ -f "$v01_report" ]] || { echo "BLOCKED: V01 report not found: $v01_report" >&2; exit 78; }
python3 - "$v01_report" <<'PY'
import json, pathlib, sys
report=json.loads(pathlib.Path(sys.argv[1]).read_text())
if report.get("schema_version") != 1: raise SystemExit("V01 report schema_version is not 1")
expected={"cross_domain_acceptance", "functional_indicators", "architecture"}
results=report.get("results", {})
if not expected <= results.keys(): raise SystemExit("V01 report is missing required result groups")
if any(not isinstance(results[key], str) or not results[key].startswith("verified_") for key in expected):
    raise SystemExit("V01 report does not contain verified result evidence")
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
