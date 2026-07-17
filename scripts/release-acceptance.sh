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

validate_release_lane_evidence() {
  local candidate_sha256=$1 installed_receipt=$2 monitor_report=$3 scenario_manifest=$4
  local failure_receipt=$5 failure_runtime_hashes=$6 inventory_output=$7
  python3 - "$candidate_sha256" "$installed_receipt" "$monitor_report" \
    "$scenario_manifest" "$failure_receipt" "$failure_runtime_hashes" "$inventory_output" <<'PY'
import json, pathlib, re, sys

candidate, installed_path, monitor_path, scenario_manifest_path, failure_path, runtime_hashes_json, output_path = sys.argv[1:]
checksum = re.compile(r"[0-9a-f]{64}").fullmatch

def fail(message):
    raise SystemExit(f"release evidence: {message}")

if not checksum(candidate):
    fail("candidate checksum is invalid")

def load(path, lane):
    candidate_path = pathlib.Path(path)
    if not candidate_path.is_file() or candidate_path.is_symlink():
        fail(f"{lane} receipt is missing or unsafe")
    try:
        value = json.loads(candidate_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(f"{lane} receipt is invalid JSON: {error}")
    if not isinstance(value, dict):
        fail(f"{lane} receipt is not an object")
    return value

installed = load(installed_path, "installed-host")
if installed.get("status") != "PASS" or installed.get("lane") != "disposable-installed-host":
    fail("installed-host lane did not pass")
if installed.get("candidate_sha256") != candidate:
    fail("installed-host receipt is not bound to the candidate")
if (installed.get("active_binary_sha256") != candidate or
        installed.get("post_rollback_candidate_reapplied") is not True):
    fail("installed-host lane did not leave the candidate active")
baseline = installed.get("baseline_sha256")
if not isinstance(baseline, str) or not checksum(baseline) or baseline == candidate:
    fail("installed-host baseline is invalid or not distinct")
if installed.get("distinct_release_upgrade") is not True:
    fail("installed-host lane did not prove a distinct release upgrade")

monitor = load(monitor_path, "monitor")
if monitor.get("suite") != "production" or monitor.get("status") != "PASS":
    fail("monitor production suite did not pass")
preflight = monitor.get("preflight")
if not isinstance(preflight, dict) or preflight.get("binary_sha256") != candidate:
    fail("monitor preflight binary is not the release candidate")
cases = monitor.get("cases")
expected_scenarios = {
    "project_workspace", "gmail_analysis", "subagent_research", "reconnect_resume"
}

if (not isinstance(cases, list) or
        {case.get("scenario") for case in cases if isinstance(case, dict)} != expected_scenarios or
        len(cases) != len(expected_scenarios)):
    fail("monitor scenario inventory drifted")
if any(not isinstance(case, dict) or case.get("status") != "PASS" for case in cases):
    fail("monitor inventory contains a non-passing case")
summary = monitor.get("summary")
derived_monitor_summary = {
    status: sum(case.get("status") == status for case in cases)
    for status in ("PASS", "FAIL", "BLOCKED")
}
if summary != derived_monitor_summary:
    fail("monitor summary does not match its case inventory")

required_assertions = {
    "project_workspace": {
        "known_git_root", "initial_worktree_clean", "git_head_stable",
        "worktree_restored", "scenario_artifacts_scoped", "artifact_delivery",
        "repository_analysis", "outside_write_denied",
    },
    "gmail_analysis": {
        "turn_done", "single_bounded_search", "configured_account_bound", "authorized",
        "result_schema_bounded", "metadata_only", "summary_bounded_and_redacted",
        "durable_event_evidence", "live_test_account_configured",
        "account_binding_attested", "wire_schema_attested",
    },
    "subagent_research": {
        "authoritative_turn_done", "unique_initial_event_receipt", "session_id_recorded",
        "exact_agent_lifecycle_tools", "unique_call_ids", "tool_results_accounted",
        "two_distinct_spawned_agents", "first_agent_progress_listed",
        "mailbox_delivered_to_first_agent", "first_agent_terminal_result",
        "agent_result_marker_hash", "parent_text_promoted_result",
        "parent_journal_promoted_result", "second_agent_cancelled",
        "daemon_restart_command", "daemon_process_changed",
        "daemon_start_timestamp_changed", "same_candidate_binary",
        "unique_post_restart_event_receipt", "post_restart_call_ids_unique",
        "both_agents_requeried", "terminal_states_persisted",
        "result_hashes_persisted", "same_session_recovered",
    },
    "reconnect_resume": {
        "initial_turn_done", "initial_event_evidence_durable", "structured_long_output",
        "final_marker_recorded", "real_page_scroll", "returned_to_final_view",
        "tui_reconnected", "post_reconnect_turn_done",
        "reconnect_event_evidence_durable", "same_session_id",
        "resume_record_count", "final_answer_persisted",
    },
}
by_scenario = {case["scenario"]: case for case in cases}
for scenario, required in required_assertions.items():
    case = by_scenario[scenario]
    if case.get("failure") is not None:
        fail(f"monitor {scenario} retained a failure")
    assertions = case.get("assertions")
    if not isinstance(assertions, list) or not assertions:
        fail(f"monitor {scenario} assertions are missing")
    names = [item.get("name") for item in assertions if isinstance(item, dict)]
    if len(names) != len(assertions) or len(names) != len(set(names)):
        fail(f"monitor {scenario} assertion inventory is invalid")
    if any(item.get("passed") is not True for item in assertions):
        fail(f"monitor {scenario} contains a failed assertion")
    if not required.issubset(set(names)):
        fail(f"monitor {scenario} required assertions are missing")

def event_receipt(value, label, path_required=True):
    if not isinstance(value, dict):
        fail(f"{label} event receipt is missing")
    if path_required and (not isinstance(value.get("path"), str) or not value["path"].startswith("/")):
        fail(f"{label} event path is invalid")
    if not isinstance(value.get("event_count"), int) or isinstance(value["event_count"], bool) or value["event_count"] <= 0:
        fail(f"{label} event count is invalid")
    if not isinstance(value.get("size_bytes"), int) or isinstance(value["size_bytes"], bool) or value["size_bytes"] <= 0:
        fail(f"{label} event size is invalid")
    if not isinstance(value.get("sha256"), str) or not checksum(value["sha256"]):
        fail(f"{label} event checksum is invalid")
    if path_required and (value.get("mode") != "0600" or not isinstance(value.get("uid"), int)
                          or isinstance(value["uid"], bool) or value["uid"] < 0):
        fail(f"{label} event ownership receipt is invalid")
    return value

project = by_scenario["project_workspace"].get("evidence")
if not isinstance(project, dict): fail("project evidence is missing")
before, after = project.get("git_before"), project.get("git_after")
if (not isinstance(before, dict) or not isinstance(after, dict)
        or not isinstance(before.get("head"), str) or not before["head"]
        or before.get("head") != after.get("head")
        or before.get("status") != "" or after.get("status") != ""):
    fail("project Git evidence is invalid")
for name in ("delivery", "analysis", "boundary"):
    child = project.get(name)
    if not isinstance(child, dict) or child.get("status") != "PASS":
        fail(f"project {name} evidence did not pass")
    event_receipt(child.get("evidence", {}).get("event"), f"project:{name}")

gmail = by_scenario["gmail_analysis"]
gmail_evidence = gmail.get("evidence")
if (gmail.get("authorization_state") != "authorized" or not isinstance(gmail_evidence, dict)
        or gmail_evidence.get("metadata_only") is not True
        or gmail_evidence.get("item_limit") != 10
        or not isinstance(gmail_evidence.get("item_count"), int)
        or gmail_evidence["item_count"] > 10):
    fail("Gmail bounded authorization evidence is invalid")
event_receipt({"event_count": gmail_evidence.get("event_count"),
               "size_bytes": gmail_evidence.get("event_size_bytes"),
               "sha256": gmail_evidence.get("event_sha256")}, "gmail", False)
for field in ("summary_sha256", "result_sha256"):
    if not isinstance(gmail_evidence.get(field), str) or not checksum(gmail_evidence[field]):
        fail(f"Gmail {field} is invalid")

subagent = by_scenario["subagent_research"].get("evidence")
if (not isinstance(subagent, dict) or not subagent.get("session_id")
        or subagent.get("journal_promoted") is not True
        or subagent.get("lifecycle", {}).get("result_promoted_to_parent") is not True
        or subagent.get("restart", {}).get("process_changed") is not True
        or subagent.get("restart", {}).get("start_timestamp_changed") is not True
        or subagent.get("restart", {}).get("same_candidate_binary") is not True
        or subagent.get("post_restart_agents", {}).get("terminal_states_persisted") is not True
        or subagent.get("post_restart_agents", {}).get("result_hashes_persisted") is not True):
    fail("SubAgent lifecycle or restart evidence is invalid")
event_receipt(subagent.get("initial_event"), "subagent:initial")
event_receipt(subagent.get("post_restart_event"), "subagent:post_restart")

reconnect = by_scenario["reconnect_resume"].get("evidence")
if (not isinstance(reconnect, dict) or not reconnect.get("session_id")
        or reconnect.get("resumed_session_id") != reconnect.get("session_id")
        or not isinstance(reconnect.get("final_sha256"), str) or not checksum(reconnect["final_sha256"])
        or not isinstance(reconnect.get("final_bytes"), int) or reconnect["final_bytes"] <= 1000
        or not isinstance(reconnect.get("final_lines"), int) or reconnect["final_lines"] < 60
        or not isinstance(reconnect.get("journal_entries"), int) or reconnect["journal_entries"] <= 0):
    fail("reconnect persistence evidence is invalid")
event_receipt(reconnect.get("initial_event"), "reconnect:initial")
event_receipt(reconnect.get("reconnect_event"), "reconnect:reconnected")

scenario_manifest = load(scenario_manifest_path, "production-scenario-events")
expected_event_ids = {
    "project_workspace:delivery", "project_workspace:analysis", "project_workspace:boundary",
    "gmail_analysis:gmail", "subagent_research:initial", "subagent_research:post_restart",
    "reconnect_resume:initial", "reconnect_resume:reconnected",
}
manifest_events = scenario_manifest.get("events")
if (scenario_manifest.get("schema_version") != 1 or scenario_manifest.get("status") != "PASS"
        or not isinstance(manifest_events, list)
        or {item.get("id") for item in manifest_events if isinstance(item, dict)} != expected_event_ids
        or len(manifest_events) != len(expected_event_ids)):
    fail("production scenario event manifest is invalid")
if any(item.get("mode") != "0600" or not checksum(item.get("sha256", ""))
       or not isinstance(item.get("size_bytes"), int) or item["size_bytes"] <= 0
       or not isinstance(item.get("event_count"), int) or item["event_count"] <= 0
       or not isinstance(item.get("uid"), int) or item["uid"] < 0
       or not isinstance(item.get("path"), str) or not item["path"].startswith("production-workspace/")
       for item in manifest_events):
    fail("production scenario event manifest entries are invalid")
expected_receipts = {
    "project_workspace:delivery": project["delivery"]["evidence"]["event"],
    "project_workspace:analysis": project["analysis"]["evidence"]["event"],
    "project_workspace:boundary": project["boundary"]["evidence"]["event"],
    "gmail_analysis:gmail": {
        "sha256": gmail_evidence["event_sha256"],
        "size_bytes": gmail_evidence["event_size_bytes"],
        "event_count": gmail_evidence["event_count"],
    },
    "subagent_research:initial": subagent["initial_event"],
    "subagent_research:post_restart": subagent["post_restart_event"],
    "reconnect_resume:initial": reconnect["initial_event"],
    "reconnect_resume:reconnected": reconnect["reconnect_event"],
}
for item in manifest_events:
    expected = expected_receipts[item["id"]]
    if any(item.get(field) != expected.get(field)
           for field in ("sha256", "size_bytes", "event_count")):
        fail(f"production scenario event manifest drifted for {item['id']}")
    if expected.get("uid") is not None and item.get("uid") != expected.get("uid"):
        fail(f"production scenario event owner drifted for {item['id']}")

failure = load(failure_path, "failure-matrix")
if failure.get("status") != "PASS" or failure.get("lane") != "disposable-installed-host":
    fail("failure-matrix lane did not pass")
if failure.get("candidate_sha256") != candidate:
    fail("failure-matrix receipt is not bound to the candidate")
ignored_failure_cases = failure.get("ignored_cases")
if (not isinstance(ignored_failure_cases, int) or isinstance(ignored_failure_cases, bool)
        or ignored_failure_cases < 0):
    fail("failure-matrix ignored-case inventory is invalid")
ignored_failure_inventory = failure.get("ignored_inventory")
if (not isinstance(ignored_failure_inventory, list)
        or len(ignored_failure_inventory) != ignored_failure_cases):
    fail("failure-matrix ignored count does not match its inventory")
provenance = failure.get("runtime_provenance")
if (not isinstance(provenance, dict) or provenance.get("boundary") != "per-user-runtime"
        or provenance.get("candidate_sha256") != candidate):
    fail("failure-matrix runtime provenance is missing")
for runtime in ("machine", "user"):
    value = provenance.get(runtime)
    if (not isinstance(value, dict) or not isinstance(value.get("pid"), int)
            or isinstance(value.get("pid"), bool) or value["pid"] <= 0):
        fail(f"failure-matrix {runtime} runtime provenance is invalid")
try:
    runtime_hashes = json.loads(runtime_hashes_json)
except json.JSONDecodeError as error:
    fail(f"failure runtime checksum evidence is invalid: {error}")
if runtime_hashes != {"machine": candidate, "user": candidate}:
    fail("failure-matrix runtime processes are not the release candidate")

inventory = [
    {"id": "v01", "status": "PASS"},
    {"id": "installed-host", "status": installed["status"]},
    *({"id": f"monitor:{case['scenario']}", "status": case["status"]} for case in cases),
    {"id": "failure-matrix", "status": failure["status"]},
]
inventory.extend(
    {"id": f"failure-matrix:ignored:{index + 1}", "status": "IGNORED", "evidence": item}
    for index, item in enumerate(ignored_failure_inventory)
)
ignored_statuses = {"IGNORED", "SKIP", "SKIPPED"}
result = {
    "schema_version": 1,
    "cases": inventory,
    "summary": {
        "total": len(inventory),
        "passed": sum(case["status"] == "PASS" for case in inventory),
        "failed": sum(case["status"] == "FAIL" for case in inventory),
        "blocked": sum(case["status"] == "BLOCKED" for case in inventory),
        "ignored": sum(case["status"] in ignored_statuses for case in inventory),
    },
}
if any(result["summary"][field] for field in ("failed", "blocked", "ignored")):
    fail("release case inventory is not fully passing")
pathlib.Path(output_path).write_text(json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n")
PY
}

write_scenario_evidence_manifest() {
  local monitor_report=$1 workspace=$2 copied_root=$3 expected_uid=$4 output=$5
  python3 - "$monitor_report" "$workspace" "$copied_root" "$expected_uid" "$output" <<'PY'
import hashlib, json, pathlib, stat, sys

monitor_path, workspace_value, copied_value, uid_value, output_value = sys.argv[1:]
workspace = pathlib.Path(workspace_value)
scenario_root = workspace / ".scenario-runs"
copied_root = pathlib.Path(copied_value)
expected_uid = int(uid_value)

def fail(message):
    raise SystemExit(f"scenario evidence manifest: {message}")

try:
    monitor = json.loads(pathlib.Path(monitor_path).read_text())
except (OSError, json.JSONDecodeError) as error:
    fail(f"monitor report is invalid: {error}")
cases = monitor.get("cases")
if not isinstance(cases, list): fail("monitor cases are missing")
by_name = {case.get("scenario"): case for case in cases if isinstance(case, dict)}
try:
    project = by_name["project_workspace"]["evidence"]
    gmail = by_name["gmail_analysis"]["evidence"]
    subagent = by_name["subagent_research"]["evidence"]
    reconnect = by_name["reconnect_resume"]["evidence"]
    receipts = [
        ("project_workspace:delivery", project["delivery"]["evidence"]["event"]),
        ("project_workspace:analysis", project["analysis"]["evidence"]["event"]),
        ("project_workspace:boundary", project["boundary"]["evidence"]["event"]),
        ("gmail_analysis:gmail", {
            "sha256": gmail["event_sha256"], "size_bytes": gmail["event_size_bytes"],
            "event_count": gmail["event_count"],
        }),
        ("subagent_research:initial", subagent["initial_event"]),
        ("subagent_research:post_restart", subagent["post_restart_event"]),
        ("reconnect_resume:initial", reconnect["initial_event"]),
        ("reconnect_resume:reconnected", reconnect["reconnect_event"]),
    ]
except (KeyError, TypeError) as error:
    fail(f"required event receipt is missing: {error}")

def receipt_shape(receipt, label):
    if (not isinstance(receipt, dict) or not isinstance(receipt.get("sha256"), str)
            or len(receipt["sha256"]) != 64
            or not isinstance(receipt.get("size_bytes"), int)
            or isinstance(receipt["size_bytes"], bool) or receipt["size_bytes"] <= 0
            or not isinstance(receipt.get("event_count"), int)
            or isinstance(receipt["event_count"], bool) or receipt["event_count"] <= 0):
        fail(f"{label} receipt shape is invalid")

def safe_files():
    result = []
    for path in copied_root.rglob("*"):
        metadata = path.lstat()
        if stat.S_ISREG(metadata.st_mode) and not path.is_symlink():
            result.append(path)
    return result

available = safe_files()
used = set()
events = []
for identifier, receipt in receipts:
    receipt_shape(receipt, identifier)
    source_path = receipt.get("path")
    if isinstance(source_path, str):
        try:
            relative = pathlib.Path(source_path).relative_to(scenario_root)
        except ValueError:
            fail(f"{identifier} source path escaped the scenario root")
        if not relative.parts or ".." in relative.parts:
            fail(f"{identifier} source path escaped the scenario root")
        candidate = copied_root / relative
        candidates = [candidate]
    else:
        candidates = []
        for candidate in available:
            content = candidate.read_bytes()
            if (len(content) == receipt["size_bytes"]
                    and hashlib.sha256(content).hexdigest() == receipt["sha256"]):
                candidates.append(candidate)
    candidates = [candidate for candidate in candidates if candidate not in used]
    if len(candidates) != 1:
        fail(f"{identifier} did not map uniquely to copied evidence")
    path = candidates[0]
    try:
        relative = path.relative_to(copied_root)
    except ValueError:
        fail(f"{identifier} copied path escaped the artifact root")
    if not relative.parts or ".." in relative.parts:
        fail(f"{identifier} copied path escaped the artifact root")
    if any((copied_root / pathlib.Path(*relative.parts[:index])).is_symlink()
           for index in range(1, len(relative.parts) + 1)):
        fail(f"{identifier} copied path contains a symlink")
    metadata = path.lstat()
    mode = stat.S_IMODE(metadata.st_mode)
    if (not stat.S_ISREG(metadata.st_mode) or path.is_symlink()
            or metadata.st_uid != expected_uid or mode != 0o600):
        fail(f"{identifier} copied ownership or mode is unsafe")
    content = path.read_bytes()
    try:
        records = [json.loads(line) for line in content.decode("utf-8").splitlines()]
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"{identifier} copied JSONL is invalid: {error}")
    digest = hashlib.sha256(content).hexdigest()
    if (not records or any(not isinstance(record, dict) for record in records)
            or len(records) != receipt["event_count"]
            or len(content) != receipt["size_bytes"] or digest != receipt["sha256"]):
        fail(f"{identifier} copied receipt does not match the monitor report")
    if source_path is not None and (receipt.get("uid") != expected_uid or receipt.get("mode") != "0600"):
        fail(f"{identifier} original ownership receipt is invalid")
    used.add(path)
    events.append({
        "id": identifier,
        "path": f"production-workspace/{relative.as_posix()}",
        "sha256": digest,
        "size_bytes": len(content),
        "event_count": len(records),
        "uid": metadata.st_uid,
        "mode": "0600",
    })

result = {"schema_version": 1, "status": "PASS", "events": events}
pathlib.Path(output_value).write_text(
    json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n"
)
PY
}

write_lane_evidence_manifest() {
  local candidate_sha256=$1 output=$2
  shift 2
  python3 - "$candidate_sha256" "$output" "$@" <<'PY'
import hashlib, json, pathlib, sys

candidate, output, *paths = sys.argv[1:]
names = (
    "v01", "v01_recipe", "migration_matrix", "installed_host", "candidate_activation", "monitor",
    "scenario_events", "failure_matrix", "architecture_gate", "dependency_tree", "case_inventory",
)
references = (
    "v01-acceptance.json",
    "guest/v01-acceptance-recipe.json",
    "guest/migration-matrix-receipt.json",
    "guest/installed-host/operator-receipt.json",
    "guest/candidate-activation-receipt.json",
    "production-scenarios.json",
    "guest/production-scenario-events.json",
    "guest/failure-matrix/operator-receipt.json",
    "guest/architecture-gate-receipt.json",
    "dependency-tree.txt",
    "release-case-inventory.json",
)
if len(paths) != len(names):
    raise SystemExit("release evidence: lane inventory is incomplete")
evidence = {}
for name, reference, raw_path in zip(names, references, paths):
    path = pathlib.Path(raw_path)
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"release evidence: unsafe lane artifact: {name}")
    evidence[name] = {
        "path": reference,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }
result = {"schema_version": 1, "candidate_sha256": candidate, "evidence": evidence}
pathlib.Path(output).write_text(json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n")
PY
}

if [[ ${1:-} == --validate-v01-report ]]; then
  [[ $# -eq 2 ]] || { echo "usage: $0 --validate-v01-report FILE" >&2; exit 64; }
  validate_v01_report "$2"
  exit
fi
if [[ ${1:-} == --validate-release-lane-evidence ]]; then
  [[ $# -eq 8 ]] || {
    echo "usage: $0 --validate-release-lane-evidence CANDIDATE_SHA INSTALLED_JSON MONITOR_JSON SCENARIO_MANIFEST FAILURE_JSON RUNTIME_HASHES_JSON INVENTORY_JSON" >&2
    exit 64
  }
  validate_release_lane_evidence "$2" "$3" "$4" "$5" "$6" "$7" "$8"
  exit
fi
if [[ ${1:-} == --write-scenario-evidence-manifest ]]; then
  [[ $# -eq 6 ]] || {
    echo "usage: $0 --write-scenario-evidence-manifest MONITOR_JSON WORKSPACE COPIED_ROOT UID OUTPUT" >&2
    exit 64
  }
  write_scenario_evidence_manifest "$2" "$3" "$4" "$5" "$6"
  exit
fi
if [[ ${1:-} == --write-lane-evidence-manifest ]]; then
  [[ $# -eq 14 ]] || {
    echo "usage: $0 --write-lane-evidence-manifest CANDIDATE_SHA OUTPUT ELEVEN_LANE_FILES..." >&2
    exit 64
  }
  write_lane_evidence_manifest "$2" "$3" "${@:4}"
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
candidate=${ALETHEON_RELEASE_BINARY:-}
[[ -n "$candidate" && -x "$candidate" && -f "$candidate" && ! -L "$candidate" ]] || {
  echo "BLOCKED: ALETHEON_RELEASE_BINARY must be a safe executable candidate" >&2; exit 78;
}
candidate_sha256=$(sha256sum -- "$candidate" | cut -d' ' -f1)
just --justfile "$repo_root/justfile" acceptance
v01_report=${ALETHEON_V01_ACCEPTANCE_REPORT:-"$repo_root/target/acceptance/acceptance.json"}
validate_v01_report "$v01_report"
v01_bundle="$artifacts/v01-acceptance.json"
install -m 0600 "$v01_report" "$v01_bundle"
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
migration_receipt="$guest_artifacts/migration-matrix-receipt.json"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg matrix_sha256 "$(sha256sum "$repo_root/config/release/migration-matrix.toml" | cut -d' ' -f1)" \
  '{schema_version:1,status:"PASS",completed_utc:$completed_utc,matrix_sha256:$matrix_sha256}' \
  >"$migration_receipt"
ALETHEON_RELEASE_ARTIFACTS="$guest_artifacts/installed-host" \
  "$repo_root/tests/production/install_upgrade_restart.sh"
source "$repo_root/tests/production/lib/installed_host.sh"
# The installed-host drill proves rollback and then reapplies the candidate
# through the production upgrade path. Refuse to start live workflows unless
# both its receipt and the active executable identify that candidate.
installed_receipt="$guest_artifacts/installed-host/operator-receipt.json"
jq -e --arg candidate "$candidate_sha256" \
  '.status == "PASS" and .candidate_sha256 == $candidate
   and .active_binary_sha256 == $candidate
   and .post_rollback_candidate_reapplied == true' \
  "$installed_receipt" >/dev/null
[[ $(sha256sum /usr/bin/aletheon | cut -d' ' -f1) == "$candidate_sha256" ]] || {
  echo "installed-host lane did not leave the candidate active" >&2; exit 1;
}
candidate_activation_receipt="$guest_artifacts/candidate-activation-receipt.json"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg candidate_sha256 "$candidate_sha256" \
  '{schema_version:1,status:"PASS",completed_utc:$completed_utc,candidate_sha256:$candidate_sha256,
    source:"installed-host-production-upgrade",system_unit:"aletheon-core.service",user_unit:"aletheon.service"}' \
  >"$candidate_activation_receipt"
production_user=${ALETHEON_PRODUCTION_USER:-${ALETHEON_TEST_USER_A:-}}
[[ -n "$production_user" ]] || {
  echo "BLOCKED: ALETHEON_PRODUCTION_USER or ALETHEON_TEST_USER_A is required" >&2; exit 78;
}
case " $ALETHEON_TEST_USER_A $ALETHEON_TEST_USER_B " in
  *" $production_user "*) ;;
  *) echo "BLOCKED: production scenario user was not admitted by the installed-host lane" >&2; exit 78 ;;
esac
production_uid=$(installed_user_uid "$production_user")
production_gid=$(id -g "$production_user")
production_socket=$(installed_user_socket "$production_user")
candidate_source_commit=$(git -C "$repo_root" rev-parse HEAD)
production_workspace=$(mktemp -d "/var/tmp/aletheon-production-workspace.${production_uid}.XXXXXX")
rmdir -- "$production_workspace"
scenario_event_manifest="$guest_artifacts/production-scenario-events.json"
(
  worktree_registered=0
  cleanup_production_worktree() {
    local status=$? cleanup_status=0
    trap - EXIT
    set +e
    if [[ -e "$production_workspace/.git" ]]; then worktree_registered=1; fi
    install -d -m 0700 "$guest_artifacts/production-workspace"
    if ((worktree_registered)) && [[ -d "$production_workspace/.scenario-runs" ]]; then
      cp -a -- "$production_workspace/.scenario-runs/." \
        "$guest_artifacts/production-workspace/" || cleanup_status=$?
    fi
    if ((status == 0 && cleanup_status == 0)); then
      write_scenario_evidence_manifest "$artifacts/production-scenarios.json" \
        "$production_workspace" "$guest_artifacts/production-workspace" \
        "$production_uid" "$scenario_event_manifest" || cleanup_status=$?
    fi
    cd /
    if ((worktree_registered)); then
      git -C "$repo_root" worktree remove --force "$production_workspace" || cleanup_status=$?
      git -C "$repo_root" worktree prune || cleanup_status=$?
    fi
    if ((status == 0 && cleanup_status != 0)); then status=$cleanup_status; fi
    exit "$status"
  }
  trap cleanup_production_worktree EXIT
  git -C "$repo_root" worktree add --detach "$production_workspace" "$candidate_source_commit"
  worktree_registered=1
  chown -R "$production_uid:$production_gid" "$production_workspace"
  chmod 0700 "$production_workspace"
  [[ $(stat -c '%u:%g' "$production_workspace") == "$production_uid:$production_gid" ]] || {
    echo "production worktree is not owned by the admitted user" >&2; exit 1;
  }
  [[ $(git -c "safe.directory=$production_workspace" -C "$production_workspace" rev-parse HEAD) == "$candidate_source_commit" ]] || {
    echo "production worktree is not bound to the candidate source commit" >&2; exit 1;
  }
  printf '%s\n' "$production_workspace" >"$guest_artifacts/production-workspace-path.txt"
  jq -n --arg path "$production_workspace" --arg source_commit "$candidate_source_commit" \
    --arg user "$production_user" --argjson uid "$production_uid" --argjson gid "$production_gid" \
    '{schema_version:1,status:"PASS",path:$path,source_commit:$source_commit,
      admitted_user:$user,uid:$uid,gid:$gid,detached:true}' \
    >"$guest_artifacts/production-worktree-receipt.json"
  cd "$production_workspace/tools/aletheon-monitor"
  PYTHONDONTWRITEBYTECODE=1 python3 -m pytest -q tests
  run_as_installed_user "$production_user" env \
    ALETHEON_SOCKET="$production_socket" \
    ALETHEON_PRODUCTION_WORKSPACE="$production_workspace" \
    ALETHEON_PRODUCTION_GMAIL_ACCOUNT="${ALETHEON_PRODUCTION_GMAIL_ACCOUNT:-}" \
    PYTHONDONTWRITEBYTECODE=1 \
    python3 -m src.__main__ scenario --suite production --source-root "$production_workspace" \
    | tee "$artifacts/production-scenarios.json"
  jq -e --arg source_commit "$candidate_source_commit" --arg workspace "$production_workspace" '
    .preflight.source_commit == $source_commit and .preflight.source_root == $workspace and
    ([.cases[] | select(.scenario == "project_workspace")] | length) == 1 and
    ([.cases[] | select(.scenario == "project_workspace")][0] as $project |
      $project.status == "PASS" and
      $project.evidence.workspace == $workspace and
      $project.evidence.git_before.head == $source_commit and
      $project.evidence.git_after.head == $source_commit and
      $project.evidence.git_before.status == "" and
      $project.evidence.git_after.status == "")
  ' "$artifacts/production-scenarios.json" >/dev/null
)
ALETHEON_RELEASE_ARTIFACTS="$guest_artifacts" ALETHEON_V01_ACCEPTANCE_REPORT="$v01_report" \
  ALETHEON_V01_RECIPE_RECEIPT="$v01_recipe_receipt" \
  "$repo_root/tests/production/failure_matrix.sh"
"$repo_root/scripts/architecture-check.sh"
architecture_receipt="$guest_artifacts/architecture-gate-receipt.json"
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg commit "$(git -C "$repo_root" rev-parse HEAD)" \
  '{schema_version:1,status:"PASS",completed_utc:$completed_utc,command:"scripts/architecture-check.sh",commit:$commit}' \
  >"$architecture_receipt"
cargo tree --workspace --edges normal >"$artifacts/dependency-tree.txt"

monitor_report="$artifacts/production-scenarios.json"
failure_receipt="$guest_artifacts/failure-matrix/operator-receipt.json"
failure_machine_pid=$(jq -er '.runtime_provenance.machine.pid' "$failure_receipt")
failure_user_pid=$(jq -er '.runtime_provenance.user.pid' "$failure_receipt")
failure_runtime_hashes=$(jq -cn \
  --arg machine "$(sha256sum "/proc/$failure_machine_pid/exe" | cut -d' ' -f1)" \
  --arg user "$(sha256sum "/proc/$failure_user_pid/exe" | cut -d' ' -f1)" \
  '{machine:$machine,user:$user}')
case_inventory="$artifacts/release-case-inventory.json"
validate_release_lane_evidence "$candidate_sha256" "$installed_receipt" "$monitor_report" \
  "$scenario_event_manifest" "$failure_receipt" "$failure_runtime_hashes" "$case_inventory"

lane_evidence="$artifacts/lane-evidence.json"
write_lane_evidence_manifest "$candidate_sha256" "$lane_evidence" \
  "$v01_bundle" "$v01_recipe_receipt" "$migration_receipt" "$installed_receipt" \
  "$candidate_activation_receipt" \
  "$monitor_report" "$scenario_event_manifest" "$failure_receipt" "$architecture_receipt" \
  "$artifacts/dependency-tree.txt" "$case_inventory"

operator=${ALETHEON_RELEASE_OPERATOR:-}
[[ -n "$operator" ]] || { echo "BLOCKED: ALETHEON_RELEASE_OPERATOR is required for the release receipt" >&2; exit 78; }
ignored_release_cases=$(jq -er '.summary.ignored' "$case_inventory")
jq -n --arg completed_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg operator "$operator" \
  --arg artifacts "$artifacts" --arg guest_artifacts "$guest_artifacts" \
  --arg candidate_sha256 "$candidate_sha256" \
  --arg lane_evidence_sha256 "$(sha256sum "$lane_evidence" | cut -d' ' -f1)" \
  --argjson lane_evidence "$(cat "$lane_evidence")" \
  --argjson release_case_inventory "$(cat "$case_inventory")" \
  --argjson ignored_release_cases "$ignored_release_cases" \
  '{schema_version:1,status:"PASS",completed_utc:$completed_utc,operator:$operator,
    artifacts:$artifacts,guest_artifacts:$guest_artifacts,guest_bundle:"guest",
    candidate_sha256:$candidate_sha256,
    external_failure_driver:"required_real_host_driver_receipted",
    failure_driver_receipt:"guest/failure-matrix/operator-receipt.json",
    lane_evidence:$lane_evidence,lane_evidence_sha256:$lane_evidence_sha256,
    release_case_inventory:$release_case_inventory,ignored_release_cases:$ignored_release_cases}' \
  >"$artifacts/operator-receipt.json"
echo "release acceptance passed: $artifacts/operator-receipt.json"
