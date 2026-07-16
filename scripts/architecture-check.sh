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
  rg -n --no-heading "$pattern" -g '*.rs' "$@" 2>/dev/null | normalize_rg "$rule" >> "$actual" || true
}

# Approved direct Tool::execute calls live in Corpus runtime. Integration tests
# and #[cfg(test)] unit-test modules may exercise raw contracts, so scan only the
# production prefix of source files.
python3 - <<'PY' >> "$actual"
from pathlib import Path
import re

approved = {
    Path("crates/corpus/src/security/runner.rs"),
    Path("crates/corpus/src/tools/tools/executor.rs"),
}
pattern = re.compile(r"\b(?:tool|exec)\.execute\(")
for root in (Path("crates/corpus"), Path("crates/executive"), Path("crates/bin")):
    for path in root.rglob("*.rs"):
        if "tests" in path.parts or path in approved:
            continue
        production = path.read_text().split("#[cfg(test)]", 1)[0]
        for line in production.splitlines():
            if pattern.search(line):
                normalized = " ".join(line.split())
                print(f"direct_tool|{path}|{normalized}")
PY
scan legacy_event 'use fabric::(envelope|primitives::comm)|\bEnvelope::' crates -g '!**/tests/**'
scan concrete_clock 'SystemClock::new\(' crates/dasein crates/agora crates/cognit crates/mnemosyne crates/metacog crates/interact -g '!**/tests/**'
scan core_systems_field '\.(runtime|domain|infra|orchestration|memory)\.' crates/executive/src crates/bin/src
scan duplicate_kernel 'executive::impl::kernel|crate::impl::kernel' crates
scan raw_process 'tokio::process::Command' crates/dasein/src crates/executive/src
# Concrete stores and registries are permitted only in the private composition root.
scan executive_store_import 'mnemosyne::.*(Store|Database)|corpus::.*(Registry|Runner)' crates/executive/src -g '!**/impl/daemon/bootstrap/**'
sort -u "$actual" -o "$actual"

# K02 deletion gate: cognitive domains receive Clock from Executive. Unit-test
# modules may use TestClock, but no production prefix may mention SystemClock.
python3 - <<'PY'
from pathlib import Path
violations = []
for domain in ("cognit", "dasein", "agora"):
    for path in (Path("crates") / domain / "src").rglob("*.rs"):
        production = path.read_text().split("#[cfg(test)]", 1)[0]
        if "SystemClock" in production:
            violations.append(str(path))
if violations:
    raise SystemExit("architecture-check: domain concrete clock bypass:\n" + "\n".join(violations))
PY

# K02 deletion gate: lifecycle tables and the retired service locator are
# private Kernel details. Executive and binaries may depend only on the opaque
# runtime API, and the old Executive-local kernel implementation must stay gone.
if rg -n 'ServicePorts|ProcessTable|OperationTable|InMemorySpaceManager|executive::.*kernel' \
  crates/executive/src crates/bin/src; then
  echo "architecture-check: production lifecycle authority escaped KernelRuntime" >&2
  exit 1
fi
if [[ -d crates/executive/src/impl/kernel ]]; then
  echo "architecture-check: retired Executive-local kernel directory exists" >&2
  exit 1
fi
if rg -n '^pub (mod table|use table::(ProcessTable|OperationTable))' \
  crates/kernel/src/process/mod.rs crates/kernel/src/operation/mod.rs; then
  echo "architecture-check: lifecycle table mutation API is public" >&2
  exit 1
fi
if rg -n '^pub (mod manager|use manager::InMemorySpaceManager)' crates/kernel/src/space/mod.rs; then
  echo "architecture-check: concrete space manager API is public" >&2
  exit 1
fi

# G03 deletion gate: Executive owns the only production AgentControlPort
# implementation. The retired spawner may remain only in its compatibility
# owner and the two bootstrap/runtime registration adapters until G04/G05.
agent_control_impls=$(python3 - <<'PY'
from pathlib import Path
for path in Path("crates").rglob("*.rs"):
    if "tests" in path.parts:
        continue
    production = path.read_text().split("#[cfg(test)]", 1)[0]
    if "impl AgentControlPort for" in production:
        print(path)
PY
)
if [[ "$agent_control_impls" != "crates/executive/src/service/agent_control/mod.rs" ]]; then
  echo "architecture-check: AgentControlPort has a non-authoritative implementation:" >&2
  echo "$agent_control_impls" >&2
  exit 1
fi
if rg -n '\bSubAgentSpawner\b' crates/corpus/src -g '*.rs'; then
  echo "architecture-check: Corpus bypasses AgentControlPort through SubAgentSpawner" >&2
  exit 1
fi
if rg -n '\bExecuteSubAgentFn\b' crates/corpus/src crates/executive/src/impl/daemon/bootstrap \
  crates/executive/src/service/agent_control -g '*.rs'; then
  echo "architecture-check: Agent execution closure bypasses AgentControlPort" >&2
  exit 1
fi
if rg -n '\.complete\(' crates/executive/src/impl/daemon/bootstrap \
  crates/executive/src/service/agent_control -g '*.rs'; then
  echo "architecture-check: Agent/bootstrap path owns a direct provider loop" >&2
  exit 1
fi
spawner_outside_compat=$(rg -l '\bSubAgentSpawner\b' crates/executive/src -g '*.rs' \
  | grep -Ev '^crates/executive/src/(core/(mod|orchestrator|sub_agent)\.rs|impl/daemon/bootstrap/runtime\.rs|impl/runtime/pi\.rs)$' || true)
if [[ -n "$spawner_outside_compat" ]]; then
  echo "architecture-check: new SubAgentSpawner dependency escaped compatibility paths:" >&2
  echo "$spawner_outside_compat" >&2
  exit 1
fi

# G06 deletion gate: child runtime projection may only admit typed candidates
# through the C01 port. It must never commit/broadcast Agora state, transition
# Dasein, or write global memory directly.
if rg -n 'AgoraOps|\.commit\(|broadcast_selection|integrate_broadcast|DaseinWorkspacePort|MemoryService|\.record\(' \
  crates/executive/src/service/agent_control/candidate_projection.rs \
  crates/executive/src/impl/runtime/native_cognit.rs; then
  echo "architecture-check: child Agent bypasses C01 candidate admission" >&2
  exit 1
fi

# G07 deletion gate: Kernel owns the registry and AgentControl owns the only
# application-level live Agent mailbox registration adapter.
mailbox_registration_outside_owner=$(rg -l 'register_process_mailbox' crates -g '*.rs' -g '!**/tests/**' \
  | grep -Ev '^crates/(kernel/src/runtime\.rs|executive/src/service/agent_control/mod\.rs)$' || true)
if [[ -n "$mailbox_registration_outside_owner" ]]; then
  echo "architecture-check: live Agent mailbox ownership escaped Kernel/AgentControl:" >&2
  echo "$mailbox_registration_outside_owner" >&2
  exit 1
fi
if rg -n '\b(InProcessMailbox|mailbox_service|mailbox_target)\b' crates/executive/src/core/sub_agent.rs; then
  echo "architecture-check: compatibility SubAgentSpawner still owns mailbox state" >&2
  exit 1
fi

# G08 production must use validated, Kernel-backed hierarchical admission;
# the compatibility semaphore constructor is restricted to focused tests.
if rg -n 'BoundedAgentAdmission::new\(' crates/executive/src/impl/daemon/bootstrap -g '*.rs'; then
  echo "architecture-check: production Agent admission bypasses typed Kernel-backed config" >&2
  exit 1
fi

# K02/X02 composition gate: Kernel remains domain-neutral. DomainPorts belongs
# to Executive, and the retired CoreSystems service locator must stay deleted.
if rg -n '^\s*(agora|dasein|cognit|mnemosyne|metacog|corpus|executive)\s*=' \
  crates/kernel/Cargo.toml || \
  rg -n '\b(agora|dasein|cognit|mnemosyne|metacog|corpus|executive)::' \
    crates/kernel/src -g '*.rs'; then
  echo "architecture-check: Kernel references an application domain" >&2
  exit 1
fi
domain_port_outside_executive=$(rg -l '\bDomainPorts\b' crates -g '*.rs' -g '!**/tests/**' \
  | grep -v '^crates/executive/' || true)
if [[ -n "$domain_port_outside_executive" ]]; then
  echo "architecture-check: DomainPorts is composed outside Executive:" >&2
  echo "$domain_port_outside_executive" >&2
  exit 1
fi
if [[ -e crates/executive/src/core/core_systems.rs ]] || \
   rg -n '\bCoreSystems\b|\.subsystems\b' crates/executive/src crates/bin/src -g '*.rs'; then
  echo "architecture-check: retired god container escaped into production" >&2
  exit 1
fi
composition_outside_bootstrap=$(rg -l '\bDaemonComposition\b' crates/executive/src -g '*.rs' \
  | grep -v '^crates/executive/src/impl/daemon/bootstrap/' || true)
if [[ -n "$composition_outside_bootstrap" ]]; then
  echo "architecture-check: private daemon composition escaped bootstrap:" >&2
  echo "$composition_outside_bootstrap" >&2
  exit 1
fi
if (( $(wc -l < crates/executive/src/impl/daemon/handler/init.rs) > 250 )); then
  echo "architecture-check: handler/init.rs is no longer a thin compatibility layer" >&2
  exit 1
fi
if (( $(wc -l < crates/executive/src/impl/daemon/bootstrap/request.rs) > 1500 )); then
  echo "architecture-check: bootstrap/request.rs exceeded its composition bound" >&2
  exit 1
fi
for stage in channels google runtime storage; do
  if (( $(wc -l < "crates/executive/src/impl/daemon/bootstrap/${stage}.rs") > 700 )); then
    echo "architecture-check: bootstrap/${stage}.rs exceeded its stage bound" >&2
    exit 1
  fi
done

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
  rg -l 'CapabilityInvoker for' crates -g '*.rs' -g '!**/tests/**' 2>/dev/null \
    | grep -v 'crates/executive/src/service/governed_capability.rs' \
    | sed 's#^#capability_path|#; s#$#|CapabilityInvoker#' || true
  rg -l '\bAdmissionRequest \{' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#capability_path|#; s#$#|manual_admission#' || true
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
