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
jq -n --arg candidate "$candidate" '
  {suite:"production",status:"PASS",preflight:{binary_sha256:$candidate},
   cases:[
     {scenario:"project_workspace",status:"PASS"},
     {scenario:"gmail_analysis",status:"PASS"},
     {scenario:"subagent_research",status:"PASS"},
     {scenario:"reconnect_resume",status:"PASS"}
   ],summary:{PASS:4,FAIL:0,BLOCKED:0}}
' >"$tmp/monitor.json"
jq -n --arg candidate "$candidate" '
  {status:"PASS",lane:"disposable-installed-host",ignored_cases:0,ignored_inventory:[],
   candidate_sha256:$candidate,
   runtime_provenance:{boundary:"per-user-runtime",candidate_sha256:$candidate,
                       machine:{pid:101},user:{pid:202}}}
' >"$tmp/failure.json"
runtime_hashes=$(jq -cn --arg candidate "$candidate" \
  '{machine:$candidate,user:$candidate}')

"$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
  "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/failure.json" \
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
  "$tmp/activation.json" "$tmp/monitor.json" "$tmp/failure.json" \
  "$tmp/architecture.json" "$tmp/dependencies.txt" "$tmp/inventory.json"
installed_sha256=$(sha256sum "$tmp/installed.json" | cut -d' ' -f1)
jq -e --arg candidate "$candidate" --arg installed_sha256 "$installed_sha256" '
  .schema_version == 1 and .candidate_sha256 == $candidate and
  (.evidence | keys | length) == 10 and
  .evidence.installed_host.path == "guest/installed-host/operator-receipt.json" and
  .evidence.installed_host.sha256 == $installed_sha256
' "$tmp/lanes.json" >/dev/null

assert_rejected() {
  local label=$1
  if "$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
    "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/failure.json" \
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

jq '.ignored_cases = 1 | .ignored_inventory = ["deliberate-skip"]' "$tmp/failure.json" >"$tmp/failure.bad"
mv "$tmp/failure.bad" "$tmp/failure.json"
assert_rejected ignored-failure-case
jq '.ignored_cases = 0 | .ignored_inventory = []' "$tmp/failure.json" >"$tmp/failure.good"
mv "$tmp/failure.good" "$tmp/failure.json"

wrong_runtime=$(jq -cn --arg candidate "$candidate" \
  '{machine:$candidate,user:("d" * 64)}')
if "$repo_root/scripts/release-acceptance.sh" --validate-release-lane-evidence \
  "$candidate" "$tmp/installed.json" "$tmp/monitor.json" "$tmp/failure.json" \
  "$wrong_runtime" "$tmp/rejected.json" >/dev/null 2>&1; then
  echo "release evidence accepted a non-candidate failure runtime" >&2
  exit 1
fi

release_script="$repo_root/scripts/release-acceptance.sh"
migration_script="$repo_root/scripts/verify-migration-matrix.sh"
grep -F 'bash "$repo_root/scripts/cargo-agent.sh" test -p mnemosyne --test gbrain_spool' \
  "$migration_script" >/dev/null
grep -F 'bash "$repo_root/scripts/cargo-agent.sh" test -p executive --test agent_control_repository' \
  "$migration_script" >/dev/null
if grep -Eq '(^|[[:space:]])cargo[[:space:]]+test' "$migration_script"; then
  echo "migration verifier bypasses the bounded Cargo wrapper" >&2
  exit 1
fi
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
