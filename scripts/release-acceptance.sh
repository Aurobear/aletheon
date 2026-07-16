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
ALETHEON_RELEASE_ARTIFACTS="$artifacts/installed-host" \
  "$repo_root/tests/production/install_upgrade_restart.sh"
(
  cd "$repo_root/tools/aletheon-monitor"
  python3 -m pytest -q tests
  python3 -m src.__main__ scenario --suite production --source-root "$repo_root" \
    | tee "$artifacts/production-scenarios.json"
)
ALETHEON_RELEASE_ARTIFACTS="$artifacts" ALETHEON_V01_ACCEPTANCE_REPORT="$v01_report" \
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
  '{status:"PASS",completed_utc:$completed_utc,operator:$operator,v01_report:$v01_report,artifacts:$artifacts,ignored_release_cases:0}' \
  >"$artifacts/operator-receipt.json"
echo "release acceptance passed: $artifacts/operator-receipt.json"
