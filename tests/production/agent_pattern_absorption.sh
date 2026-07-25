#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
runner="$repo_root/scripts/libexec/aletheon/agent-pattern-benchmark.sh"
cases="$repo_root/tests/fixtures/agent_pattern_absorption/cases.json"
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

jq -e '
  .schema_version == 1 and (.cases | length) == 7
  and ([
    "simple_read_only", "multi_file_plan", "failed_deployment_diagnosis",
    "risky_mutation_review", "deferred_tool_discovery",
    "fifty_message_fact_retention", "session_export_import"
  ] - [.cases[].id] | length) == 0
  and all(.cases[];
    (.expected_evidence | type == "array" and length > 0)
    and (.forbidden_behavior | type == "array")
    and (.maximum_attempts | type == "number" and . > 0)
    and (.mutation_allowed | type == "boolean"))
' "$cases" >/dev/null

socket="${XDG_RUNTIME_DIR:?}/aletheon/aletheon.sock"
jq -n --arg socket "$socket" '{
  schema_version:1, variant:"baseline", case_id:"simple_read_only", success:true,
  input_tokens:12, output_tokens:4, cache_hit_tokens:0, latency_ms:25, tool_calls:1,
  binary:"/usr/bin/aletheon", socket:$socket
}' >"$tmp/valid.json"
"$runner" --validate-receipt "$tmp/valid.json"

reject() {
  local name=$1 filter=$2
  jq "$filter" "$tmp/valid.json" >"$tmp/$name.json"
  if "$runner" --validate-receipt "$tmp/$name.json" >/dev/null 2>&1; then
    echo "invalid receipt accepted: $name" >&2
    exit 1
  fi
}
reject missing-field 'del(.input_tokens)'
reject unknown-case '.case_id = "unknown"'
reject development-binary '.binary = "target/release/aletheon"'
reject alternate-socket '.socket = "/tmp/aletheon.sock"'

grep -F 'installed_binary=/usr/bin/aletheon' "$runner" >/dev/null
grep -F 'official_socket="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/aletheon/aletheon.sock"' "$runner" >/dev/null
grep -F 'env -u ALETHEON_SOCKET timeout' "$runner" >/dev/null
if grep -Eq '(curl|wget).*(anthropic|openai|provider)' "$runner"; then
  echo "benchmark runner calls a provider directly" >&2
  exit 1
fi

echo "agent pattern absorption receipt contract: pass"
