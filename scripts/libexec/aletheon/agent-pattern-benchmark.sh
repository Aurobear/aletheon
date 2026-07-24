#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
cases_file="$repo_root/tests/fixtures/agent_pattern_absorption/cases.json"
artifact_root="$repo_root/target/agent-pattern-absorption"
installed_binary=/usr/bin/aletheon
official_socket="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/aletheon/aletheon.sock"

validate_receipt() {
  local receipt=$1
  [[ -f "$receipt" && ! -L "$receipt" ]] || {
    echo "receipt must be a regular file: $receipt" >&2
    return 1
  }
  jq -e --slurpfile cases "$cases_file" \
    --arg binary "$installed_binary" --arg socket "$official_socket" '
    .schema_version == 1
    and (.variant | type == "string" and length > 0)
    and (.case_id | type == "string")
    and (.success | type == "boolean")
    and (.input_tokens | type == "number" and . >= 0)
    and (.output_tokens | type == "number" and . >= 0)
    and (.cache_hit_tokens | type == "number" and . >= 0)
    and (.latency_ms | type == "number" and . >= 0)
    and (.tool_calls | type == "number" and . >= 0)
    and .binary == $binary and .socket == $socket
    and (.case_id as $id | any($cases[0].cases[]; .id == $id))
  ' "$receipt" >/dev/null
}

case_succeeds() {
  local case_json=$1 output=$2
  jq -e --arg output "$output" '
    all(.expected_evidence[] as $needle; $output | contains($needle))
    and all(.forbidden_behavior[] as $needle; ($output | contains($needle)) | not)
  ' <<<"$case_json" >/dev/null
}

run_message() {
  local session_id=$1 prompt=$2 output_file=$3 error_file=$4
  ALETHEON_BENCHMARK_METRICS=1 \
  ALETHEON_BENCHMARK_SESSION_ID="$session_id" \
  env -u ALETHEON_SOCKET timeout 150 "$installed_binary" \
    --socket "$official_socket" -C "$repo_root" -m "$prompt" \
    >"$output_file" 2>"$error_file"
}

run_case() {
  local variant=$1 run=$2 case_id=$3
  local case_json session_id case_dir output_file error_file metric_line metrics success
  case_json=$(jq -c --arg id "$case_id" '.cases[] | select(.id == $id)' "$cases_file")
  [[ -n "$case_json" ]] || { echo "unknown case: $case_id" >&2; return 2; }
  session_id="benchmark-${variant}-${case_id}-${run}-$(cat /proc/sys/kernel/random/uuid)"
  case_dir="$artifact_root/$variant/run-$run/$case_id"
  install -d -m 0700 "$case_dir"
  output_file="$case_dir/output.txt"
  error_file="$case_dir/stderr.txt"

  if [[ "$case_id" == "fifty_message_fact_retention" ]]; then
    local fact index
    fact=$(jq -r '.setup_fact' <<<"$case_json")
    for index in $(seq 1 49); do
      run_message "$session_id" \
        "Retention setup message $index of 50. Remember benchmark fact '$fact'. Reply only ACK-$index." \
        "$case_dir/setup-$index.out" "$case_dir/setup-$index.err"
    done
  fi

  run_message "$session_id" "$(jq -r '.prompt' <<<"$case_json")" "$output_file" "$error_file"
  metric_line=$(grep '^ALETHEON_BENCHMARK_METRICS=' "$error_file" | tail -1)
  [[ -n "$metric_line" ]] || { echo "missing client metrics for $case_id" >&2; return 1; }
  metrics=${metric_line#ALETHEON_BENCHMARK_METRICS=}
  if case_succeeds "$case_json" "$(cat "$output_file")"; then success=true; else success=false; fi

  jq -n --arg variant "$variant" --arg case_id "$case_id" --argjson success "$success" \
    --arg binary "$installed_binary" --arg socket "$official_socket" \
    --arg binary_sha256 "$(sha256sum "$installed_binary" | cut -d' ' -f1)" \
    --argjson metrics "$metrics" '{
      schema_version:1, variant:$variant, case_id:$case_id, success:$success,
      input_tokens:$metrics.input_tokens, output_tokens:$metrics.output_tokens,
      cache_hit_tokens:$metrics.cache_hit_tokens, latency_ms:$metrics.latency_ms,
      tool_calls:$metrics.tool_calls, binary:$binary, socket:$socket,
      binary_sha256:$binary_sha256
    }' >"$case_dir/receipt.json"
  validate_receipt "$case_dir/receipt.json"
}

usage() {
  echo "usage: $0 [--validate-receipt FILE] [--variant NAME] [--runs N] [--case ID]" >&2
}

variant=baseline
runs=3
only_case=
while (($#)); do
  case "$1" in
    --validate-receipt) validate_receipt "${2:?missing receipt}"; exit $? ;;
    --variant) variant=${2:?missing variant}; shift 2 ;;
    --runs) runs=${2:?missing runs}; shift 2 ;;
    --case) only_case=${2:?missing case}; shift 2 ;;
    *) usage; exit 2 ;;
  esac
done
[[ "$runs" =~ ^[1-9][0-9]*$ ]] || { echo "runs must be positive" >&2; exit 2; }
[[ -x "$installed_binary" && ! -L "$installed_binary" ]] || {
  echo "installed binary is unavailable: $installed_binary" >&2; exit 1;
}
[[ -S "$official_socket" ]] || { echo "official socket unavailable: $official_socket" >&2; exit 1; }

mapfile -t case_ids < <(
  if [[ -n "$only_case" ]]; then printf '%s\n' "$only_case"; else jq -r '.cases[].id' "$cases_file"; fi
)
for run in $(seq 1 "$runs"); do
  for case_id in "${case_ids[@]}"; do
    echo "benchmark variant=$variant run=$run case=$case_id"
    run_case "$variant" "$run" "$case_id"
  done
done
