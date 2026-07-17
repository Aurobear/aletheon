#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
tmp=$(mktemp -d /tmp/aletheon-release-evidence.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
candidate=$(printf 'a%.0s' {1..64})
baseline=$(printf 'b%.0s' {1..64})

jq -n --arg candidate "$candidate" --arg baseline "$baseline" '
  {status:"PASS",lane:"disposable-installed-host",candidate_sha256:$candidate,
   active_binary_sha256:$candidate,post_rollback_candidate_reapplied:true,
   baseline_sha256:$baseline,distinct_release_upgrade:true}
' >"$tmp/installed.json"
python3 - "$candidate" "$tmp" "$(id -u)" <<'PY'
import hashlib, json, pathlib, sys

candidate, root_value, uid_value = sys.argv[1:]
root = pathlib.Path(root_value)
copied = root / "production-workspace"
workspace = pathlib.Path("/candidate-worktree")
uid = int(uid_value)
copied.mkdir()

event_ids = {
    "project_workspace:delivery": "project/delivery.jsonl",
    "project_workspace:analysis": "project/analysis.jsonl",
    "project_workspace:boundary": "project/boundary.jsonl",
    "gmail_analysis:gmail": "gmail/events.jsonl",
    "subagent_research:initial": "subagent/initial.jsonl",
    "subagent_research:post_restart": "subagent/restart.jsonl",
    "reconnect_resume:initial": "reconnect/initial.jsonl",
    "reconnect_resume:reconnected": "reconnect/resumed.jsonl",
}
receipts = {}
for identifier, relative in event_ids.items():
    path = copied / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    content = (json.dumps({"type": "turn_done", "scenario": identifier}) + "\n").encode()
    path.write_bytes(content)
    path.chmod(0o600)
    receipts[identifier] = {
        "path": str(workspace / ".scenario-runs" / relative),
        "event_count": 1,
        "sha256": hashlib.sha256(content).hexdigest(),
        "size_bytes": len(content),
        "uid": uid,
        "mode": "0600",
    }

required = {
    "project_workspace": [
        "known_git_root", "initial_worktree_clean", "git_head_stable",
        "worktree_restored", "scenario_artifacts_scoped", "artifact_delivery",
        "repository_analysis", "outside_write_denied",
    ],
    "gmail_analysis": [
        "turn_done", "single_bounded_search", "configured_account_bound", "authorized",
        "result_schema_bounded", "metadata_only", "summary_bounded_and_redacted",
        "durable_event_evidence", "live_test_account_configured",
        "account_binding_attested", "wire_schema_attested",
    ],
    "subagent_research": [
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
    ],
    "reconnect_resume": [
        "initial_turn_done", "initial_event_evidence_durable", "structured_long_output",
        "final_marker_recorded", "real_page_scroll", "returned_to_final_view",
        "tui_reconnected", "post_reconnect_turn_done",
        "reconnect_event_evidence_durable", "same_session_id",
        "resume_record_count", "final_answer_persisted",
    ],
}
assertions = lambda name: [{"name": item, "passed": True} for item in required[name]]
checksum = "c" * 64
child = lambda name: {"status": "PASS", "evidence": {"event": receipts[name]}}
cases = [
    {"scenario": "project_workspace", "status": "PASS", "failure": None,
     "assertions": assertions("project_workspace"), "evidence": {
         "git_before": {"head": "source", "status": ""},
         "git_after": {"head": "source", "status": ""},
         "delivery": child("project_workspace:delivery"),
         "analysis": child("project_workspace:analysis"),
         "boundary": child("project_workspace:boundary"),
     }},
    {"scenario": "gmail_analysis", "status": "PASS", "failure": None,
     "authorization_state": "authorized", "assertions": assertions("gmail_analysis"),
     "evidence": {"metadata_only": True, "item_limit": 10, "item_count": 1,
         "summary_sha256": checksum, "result_sha256": checksum,
         "event_sha256": receipts["gmail_analysis:gmail"]["sha256"],
         "event_size_bytes": receipts["gmail_analysis:gmail"]["size_bytes"],
         "event_count": 1}},
    {"scenario": "subagent_research", "status": "PASS", "failure": None,
     "assertions": assertions("subagent_research"), "evidence": {
         "session_id": "session-agent", "journal_promoted": True,
         "initial_event": receipts["subagent_research:initial"],
         "post_restart_event": receipts["subagent_research:post_restart"],
         "lifecycle": {"result_promoted_to_parent": True},
         "restart": {"process_changed": True, "start_timestamp_changed": True,
                     "same_candidate_binary": True},
         "post_restart_agents": {"terminal_states_persisted": True,
                                 "result_hashes_persisted": True}}},
    {"scenario": "reconnect_resume", "status": "PASS", "failure": None,
     "assertions": assertions("reconnect_resume"), "evidence": {
         "session_id": "session-reconnect", "resumed_session_id": "session-reconnect",
         "final_sha256": checksum, "final_bytes": 1200, "final_lines": 60,
         "journal_entries": 4,
         "initial_event": receipts["reconnect_resume:initial"],
         "reconnect_event": receipts["reconnect_resume:reconnected"]}},
]
monitor = {"suite": "production", "status": "PASS",
           "preflight": {"binary_sha256": candidate}, "cases": cases,
           "summary": {"PASS": 4, "FAIL": 0, "BLOCKED": 0}}
(root / "monitor.json").write_text(json.dumps(monitor) + "\n")
PY
"$repo_root/scripts/release-acceptance.sh" --write-scenario-evidence-manifest \
  "$tmp/monitor.json" /candidate-worktree "$tmp/production-workspace" "$(id -u)" \
  "$tmp/scenario-events.json"
jq -e '
  .schema_version == 1 and .status == "PASS" and (.events | length) == 8 and
  ([.events[].id] | unique | length) == 8 and
  all(.events[]; .mode == "0600" and .event_count == 1 and
      (.path | startswith("production-workspace/")))
' "$tmp/scenario-events.json" >/dev/null
jq -n --arg candidate "$candidate" '
  {status:"PASS",lane:"disposable-installed-host",ignored_cases:0,ignored_inventory:[],
   candidate_sha256:$candidate,
   runtime_provenance:{boundary:"per-user-runtime",candidate_sha256:$candidate,
                       machine:{pid:101},user:{pid:202}}}
' >"$tmp/failure.json"
runtime_hashes=$(jq -cn --arg candidate "$candidate" \
  '{machine:$candidate,user:$candidate}')

"$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
  "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/scenario-events.json" "$tmp/failure.json" \
  "$runtime_hashes" "$tmp/inventory.json"
jq -e '
  .schema_version == 1 and .summary == {total:7,passed:7,failed:0,blocked:0,ignored:0}
  and ([.cases[].id] | sort) == ([
    "v01","installed-host","monitor:project_workspace","monitor:gmail_analysis",
    "monitor:subagent_research","monitor:reconnect_resume","failure-matrix"
  ] | sort)
' "$tmp/inventory.json" >/dev/null

printf 'v01\n' >"$tmp/v01.json"
printf 'recipe\n' >"$tmp/recipe.json"
printf 'migration\n' >"$tmp/migration.json"
printf 'activation\n' >"$tmp/activation.json"
printf 'architecture\n' >"$tmp/architecture.json"
printf 'dependencies\n' >"$tmp/dependencies.txt"
"$repo_root/scripts/release-acceptance.sh" --write-lane-evidence-manifest \
  "$candidate" "$tmp/lanes.json" \
  "$tmp/v01.json" "$tmp/recipe.json" "$tmp/migration.json" "$tmp/installed.json" \
  "$tmp/activation.json" "$tmp/monitor.json" "$tmp/scenario-events.json" "$tmp/failure.json" \
  "$tmp/architecture.json" "$tmp/dependencies.txt" "$tmp/inventory.json"
installed_sha256=$(sha256sum "$tmp/installed.json" | cut -d' ' -f1)
jq -e --arg candidate "$candidate" --arg installed_sha256 "$installed_sha256" '
  .schema_version == 1 and .candidate_sha256 == $candidate and
  (.evidence | keys | length) == 11 and
  .evidence.scenario_events.path == "guest/production-scenario-events.json" and
  .evidence.installed_host.path == "guest/installed-host/operator-receipt.json" and
  .evidence.installed_host.sha256 == $installed_sha256
' "$tmp/lanes.json" >/dev/null

assert_rejected() {
  local label=$1
  if "$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
    "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/scenario-events.json" "$tmp/failure.json" \
    "$runtime_hashes" "$tmp/rejected.json" >"$tmp/$label.out" 2>&1; then
    echo "release evidence unexpectedly accepted $label" >&2
    exit 1
  fi
}

jq '.preflight.binary_sha256 = ("c" * 64)' "$tmp/monitor.json" >"$tmp/monitor.bad"
mv "$tmp/monitor.bad" "$tmp/monitor.json"
assert_rejected monitor-candidate-mismatch
jq --arg candidate "$candidate" '.preflight.binary_sha256 = $candidate' \
  "$tmp/monitor.json" >"$tmp/monitor.good"
mv "$tmp/monitor.good" "$tmp/monitor.json"

jq '(.cases[] | select(.scenario == "project_workspace") | .assertions[0].passed) = false' \
  "$tmp/monitor.json" >"$tmp/monitor.bad"
mv "$tmp/monitor.bad" "$tmp/monitor.json"
assert_rejected failed-scenario-assertion
jq '(.cases[] | select(.scenario == "project_workspace") | .assertions[0].passed) = true' \
  "$tmp/monitor.json" >"$tmp/monitor.good"
mv "$tmp/monitor.good" "$tmp/monitor.json"

cp "$tmp/scenario-events.json" "$tmp/scenario-events.good"
jq '.events[0].sha256 = ("d" * 64)' "$tmp/scenario-events.json" >"$tmp/scenario-events.bad"
mv "$tmp/scenario-events.bad" "$tmp/scenario-events.json"
assert_rejected scenario-event-manifest-drift
mv "$tmp/scenario-events.good" "$tmp/scenario-events.json"

chmod 0644 "$tmp/production-workspace/project/delivery.jsonl"
if "$repo_root/scripts/release-acceptance.sh" --write-scenario-evidence-manifest \
  "$tmp/monitor.json" /candidate-worktree "$tmp/production-workspace" "$(id -u)" \
  "$tmp/rejected-events.json" >/dev/null 2>&1; then
  echo "scenario manifest accepted unsafe copied event mode" >&2
  exit 1
fi
chmod 0600 "$tmp/production-workspace/project/delivery.jsonl"

jq '(.cases[] | select(.scenario == "project_workspace") |
     .evidence.delivery.evidence.event.path) = "/candidate-worktree/.scenario-runs/../escape.jsonl"' \
  "$tmp/monitor.json" >"$tmp/monitor.bad"
if "$repo_root/scripts/release-acceptance.sh" --write-scenario-evidence-manifest \
  "$tmp/monitor.bad" /candidate-worktree "$tmp/production-workspace" "$(id -u)" \
  "$tmp/rejected-events.json" >/dev/null 2>&1; then
  echo "scenario manifest accepted an escaping source receipt" >&2
  exit 1
fi

jq '.ignored_cases = 1 | .ignored_inventory = ["deliberate-skip"]' "$tmp/failure.json" >"$tmp/failure.bad"
mv "$tmp/failure.bad" "$tmp/failure.json"
assert_rejected ignored-failure-case
jq '.ignored_cases = 0 | .ignored_inventory = []' "$tmp/failure.json" >"$tmp/failure.good"
mv "$tmp/failure.good" "$tmp/failure.json"

wrong_runtime=$(jq -cn --arg candidate "$candidate" \
  '{machine:$candidate,user:("d" * 64)}')
if "$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
  "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/scenario-events.json" "$tmp/failure.json" \
  "$wrong_runtime" "$tmp/rejected.json" >/dev/null 2>&1; then
  echo "release evidence accepted a non-candidate failure runtime" >&2
  exit 1
fi

release_script="$repo_root/scripts/release-acceptance.sh"
grep -F 'git -C "$repo_root" worktree add --detach "$production_workspace" "$candidate_source_commit"' \
  "$release_script" >/dev/null
trap_line=$(grep -nF 'trap cleanup_production_worktree EXIT' "$release_script" | cut -d: -f1)
add_line=$(grep -nF 'git -C "$repo_root" worktree add --detach "$production_workspace" "$candidate_source_commit"' \
  "$release_script" | cut -d: -f1)
[[ "$trap_line" -lt "$add_line" ]] || {
  echo "production worktree cleanup is not armed before registration" >&2
  exit 1
}
grep -F 'if [[ -e "$production_workspace/.git" ]]; then worktree_registered=1; fi' \
  "$release_script" >/dev/null
grep -F 'chown -R "$production_uid:$production_gid" "$production_workspace"' \
  "$release_script" >/dev/null
grep -F 'scenario --suite production --source-root "$production_workspace"' \
  "$release_script" >/dev/null
grep -F 'git -C "$repo_root" worktree remove --force "$production_workspace"' \
  "$release_script" >/dev/null
if grep -F 'rm -rf -- "$production_workspace"' "$release_script" >/dev/null; then
  echo "release gate bypasses git worktree cleanup" >&2
  exit 1
fi

echo "aggregate release evidence validation: pass"
