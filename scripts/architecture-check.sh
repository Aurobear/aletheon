#!/usr/bin/env bash
set -euo pipefail

ROOT=${ARCH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
ALLOW="$ROOT/config/architecture-allowlist.txt"
DEPS="$ROOT/config/architecture-dependencies.txt"
PATHS="$ROOT/config/architecture-path-inventory.txt"
mkdir -p "$ROOT/target"
actual=$(mktemp)
dep_actual=$(mktemp)
path_actual=$(mktemp)
trap 'rm -f "$actual" "$dep_actual" "$path_actual" "$actual.new" "$actual.stale" "$dep_actual.new" "$dep_actual.stale" "$path_actual.new" "$path_actual.stale"' EXIT
cd "$ROOT"

normalize_rg() {
  local rule=$1
  awk -v rule="$rule" '{
    first=index($0, ":"); rest=substr($0, first+1);
    second=index(rest, ":"); path=substr($0, 1, first-1);
    text=substr(rest, second+1); gsub(/[[:space:]]+/, " ", text);
    sub(/[[:space:]]+$/, "", text); print rule "|" path "|" text
  }' || true
}

scan() {
  local rule=$1 pattern=$2; shift 2
  rg -n --no-heading "$pattern" "$@" -g '*.rs' 2>/dev/null | normalize_rg "$rule" >> "$actual" || true
}

# Approved direct Tool::execute implementations live in Corpus runtime. Tests are
# deliberately excluded because they are allowed to exercise raw contracts.
rg -n --no-heading '\b(tool|exec)\.execute\(' crates/corpus crates/executive crates/bin -g '*.rs' -g '!**/tests/**' 2>/dev/null \
  | grep -vE '^crates/corpus/src/(security/runner|tools/tools/executor)\.rs:' \
  | normalize_rg direct_tool >> "$actual" || true
scan legacy_event 'use fabric::(envelope|primitives::comm)|\bEnvelope::' crates -g '!**/tests/**'
scan concrete_clock 'SystemClock::new\(' crates/dasein crates/agora crates/cognit crates/mnemosyne crates/metacog crates/interact -g '!**/tests/**'
scan core_systems_field '\.(runtime|domain|infra|orchestration|memory)\.' crates/executive/src crates/bin/src
scan duplicate_kernel 'executive::impl::kernel|crate::impl::kernel' crates
scan raw_process 'tokio::process::Command' crates/dasein/src crates/executive/src
scan executive_store_import 'mnemosyne::.*(Store|Database)|corpus::.*(Registry|Runner)' crates/executive/src
sort -u "$actual" -o "$actual"

if [[ ${ARCH_SKIP_DEPENDENCIES:-0} != 1 ]]; then
  cargo metadata --no-deps --format-version 1 | python3 -c '
import json,sys
data=json.load(sys.stdin)
names={p["name"] for p in data["packages"]}
for package in data["packages"]:
    for dep in package["dependencies"]:
        if dep["name"] in names:
            print("dependency|{}|{}".format(package["name"], dep["name"]))
' | sort -u > "$dep_actual"
else
  : > "$dep_actual"
fi

# Migration path inventory is symbol based and intentionally stable across line moves.
{
  rg -l 'pub struct TurnService' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#turn_path|#; s#$#|TurnService#' || true
  rg -l 'pub struct TurnPipeline' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#turn_path|#; s#$#|TurnPipeline#' || true
  rg -l 'impl TurnServices for ExecTurnServices' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#capability_path|#; s#$#|ExecTurnServices#' || true
  rg -l 'CapabilityInvoker for' crates -g '*.rs' -g '!**/tests/**' 2>/dev/null | sed 's#^#capability_path|#; s#$#|CapabilityInvoker#' || true
  rg -l 'AdmissionRequest \{' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#capability_path|#; s#$#|manual_admission#' || true
} | sort -u > "$path_actual"

compare_maximum() {
  local label=$1 baseline=$2 current=$3
  [[ -f "$baseline" ]] || { echo "architecture-check: missing $baseline" >&2; return 1; }
  local new="${current}.new" stale="${current}.stale"
  comm -23 "$current" "$baseline" > "$new"
  comm -13 "$current" "$baseline" > "$stale"
  if [[ -s "$new" ]]; then
    echo "architecture-check: new forbidden $label:" >&2
    cat "$new" >&2
    return 1
  fi
  if [[ -s "$stale" ]]; then
    echo "architecture-check: resolved $label entries (remove from baseline):"
    cat "$stale"
  fi
}

if [[ ${ARCH_UPDATE:-0} == 1 ]]; then
  cp "$actual" "$ALLOW"
  [[ ${ARCH_SKIP_DEPENDENCIES:-0} == 1 ]] || cp "$dep_actual" "$DEPS"
  cp "$path_actual" "$PATHS"
  echo 'architecture-check: baselines regenerated for inspection'
  exit 0
fi

compare_maximum findings "$ALLOW" "$actual"
[[ ${ARCH_SKIP_DEPENDENCIES:-0} == 1 ]] || compare_maximum dependencies "$DEPS" "$dep_actual"
compare_maximum migration-paths "$PATHS" "$path_actual"

if [[ -n ${ARCH_BASE_REF:-} ]]; then
  for file in config/architecture-allowlist.txt config/architecture-dependencies.txt config/architecture-path-inventory.txt; do
    if git cat-file -e "$ARCH_BASE_REF:$file" 2>/dev/null && \
       git diff --unified=0 "$ARCH_BASE_REF" -- "$file" | grep -q '^+[^+]'; then
      echo "architecture-check: $file may only lose entries" >&2
      exit 1
    fi
  done
fi

echo "architecture-check: $(wc -l < "$actual") findings, $(wc -l < "$dep_actual") dependencies, $(wc -l < "$path_actual") paths; no additions"
