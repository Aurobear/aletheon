#!/usr/bin/env bash
set -euo pipefail

# Pin C collation so `sort` (line ~158) and `comm` (compare_maximum) agree
# regardless of the caller's locale. The committed baselines under config/ are
# C-sorted; without this, an ambient UTF-8 locale makes `comm` reject the
# C-sorted baseline as "not in sorted order" and abort the gate under `set -e`,
# silently disabling all architecture enforcement.
export LC_ALL=C

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

# Q01 deletion gates: application-layer discovery belongs only to Executive,
# and only ExtensionService may translate discovery into Corpus activation.
if [[ ${ARCH_SKIP_DELETION_GATES:-0} != 1 ]]; then
if rg -n '\bconvert_event_to_turn_event\b|mpsc::channel::<(?:cognit::)?(?:Event|CognitiveStreamEvent)>' \
  crates/executive/src -g '*.rs'; then
  echo "architecture-check: Executive reintroduced the Cognit event conversion bridge" >&2
  exit 1
fi
if rg -n '\b(?:EventJournal|SessionEvent)\b|impl/session/journal' \
  crates/executive/src crates/executive/tests -g '*.rs'; then
  echo "architecture-check: Executive reintroduced a parallel Session event journal" >&2
  exit 1
fi
if rg -n '\bCommunicationBus\b' crates -g '*.rs' \
  | grep -v '^crates/fabric/'; then
  echo "architecture-check: production domains imported Fabric's legacy CommunicationBus" >&2
  exit 1
fi
if [[ -d crates/corpus/src/tools/hooks || -d crates/corpus/src/tools/skills ]]; then
  echo "architecture-check: parallel Corpus hook/skill trees were restored" >&2
  exit 1
fi
if [[ -d crates/dasein/src/impl/hook || -e crates/dasein/src/bridge/hook.rs ]]; then
  echo "architecture-check: Dasein restored an executable hook runtime" >&2
  exit 1
fi
if rg -n '^\s*pub fn (?:care_mut|boundary_mut|narrative|attention|mutate|add_care|remove_care|adjust_weight|record_outcome|adjust_from_outcome|assert|negate|add_possibility|add_entity|remove_entity|add_edge|update_readiness|update_readiness_if|set_ultimate_concern|adjust_for_mood|add_concern|remove_concern|update_fallenness|update_projection|choose_projection|ingest|passive_synthesize|update_protentions_from_patterns)\b|^\s*pub rhythm:' \
  crates/dasein/src/core/mod.rs crates/dasein/src/core/care.rs \
  crates/dasein/src/core/identity.rs crates/dasein/src/dasein/self_model.rs \
  crates/dasein/src/dasein/care_structure.rs crates/dasein/src/dasein/bewandtnis.rs \
  crates/dasein/src/dasein/temporality.rs; then
  echo "architecture-check: Dasein exposes a raw state mutator outside its reducer" >&2
  exit 1
fi
if rg -n '\b(AppConfig|load_layered)\b|ALETHEON__|/etc/aletheon/config\.toml' \
  crates/cognit/src crates/corpus/src crates/mnemosyne/src crates/dasein/src \
  crates/agora/src -g '*.rs'; then
  echo "architecture-check: application config loading escaped Executive" >&2
  exit 1
fi
extension_activation_outside_owner=$(rg -l '\bActivationRequest\b' \
  crates/executive/src crates/bin/src -g '*.rs' \
  | grep -v '^crates/executive/src/service/extension_service.rs$' || true)
if [[ -n "$extension_activation_outside_owner" ]]; then
  echo "architecture-check: extension activation bypasses Executive ExtensionService:" >&2
  echo "$extension_activation_outside_owner" >&2
  exit 1
fi
if [[ ! -s config/schema/aletheon-config.schema.json ]]; then
  echo "architecture-check: checked-in application config schema is missing" >&2
  exit 1
fi

# Q02 deletion gates: Interact and Bin may depend on Fabric protocol types, while
# domain construction belongs to Executive/Corpus composition.
if rg -n '^\s*(aletheon-kernel|corpus)\s*=' crates/interact/Cargo.toml || \
   rg -n '\b(aletheon_kernel|corpus)::|use\s+(aletheon_kernel|corpus)\b' \
     crates/interact/src -g '*.rs'; then
  echo "architecture-check: Interact imports Kernel or Corpus" >&2
  exit 1
fi
if rg -n '\b(aletheon_kernel|corpus|cognit|mnemosyne|dasein|agora|metacog)\s*=' \
     crates/bin/Cargo.toml || \
   rg -n '\b(ExecSessionBuilder|TurnRequest|RuntimeHost|KernelRuntime|ToolRegistry)\b|\b(corpus|cognit|mnemosyne|dasein|agora|metacog)::' \
     crates/bin/src -g '*.rs'; then
  echo "architecture-check: Bin owns domain or runtime construction" >&2
  exit 1
fi
if rg -n '"jsonrpc"\s*:' crates/interact/src -g '*.rs' \
  -g '!tui/test_infra.rs'; then
  echo "architecture-check: Interact manually constructs JSON-RPC envelopes" >&2
  exit 1
fi
for required in \
  crates/fabric/src/protocol/client.rs \
  crates/interact/src/tui/reducer.rs \
  crates/bin/src/lib.rs; do
  if [[ ! -s "$required" ]]; then
    echo "architecture-check: missing Q02 boundary: $required" >&2
    exit 1
  fi
done
fi

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
scan core_systems_field '\.(runtime|domain|infra|orchestration|memory)\.' crates/executive/src crates/bin/src \
  -g '!**/service/admin_service.rs' -g '!**/service/post_turn_projection.rs'
scan duplicate_kernel 'executive::impl::kernel|crate::impl::kernel' crates
scan raw_process 'tokio::process::Command' crates/dasein/src crates/executive/src
# Concrete stores and registries are permitted only in private composition roots.
# Test modules are not production dependencies, so inspect only the production prefix.
python3 - <<'PY' >> "$actual"
from pathlib import Path
import re

pattern = re.compile(r"mnemosyne::.*(?:Store|Database)|corpus::.*(?:Registry|Runner)")
for path in Path("crates/executive/src").rglob("*.rs"):
    name = str(path)
    if (
        "/impl/daemon/bootstrap/" in name
        or name == "crates/executive/src/impl/exec_corpus.rs"
        or name == "crates/executive/src/service/conscious_workspace.rs"
    ):
        continue
    production = path.read_text().split("#[cfg(test)]", 1)[0]
    for line in production.splitlines():
        if pattern.search(line):
            normalized = re.sub(r"\s+", " ", line).rstrip()
            print(f"executive_store_import|{path}|{normalized}")
PY
sort -u "$actual" -o "$actual"

# F01 deletion gate: request, turn and goal paths retain domain facades only.
# Concrete construction remains allowed in private bootstrap and CLI composition.
if [[ ${ARCH_SKIP_DELETION_GATES:-0} != 1 ]]; then
python3 - <<'PY'
from pathlib import Path

files = [
    "crates/executive/src/impl/daemon/handler/mod.rs",
    "crates/executive/src/impl/daemon/handler/init.rs",
    "crates/executive/src/impl/daemon/handler/ports.rs",
    "crates/executive/src/impl/daemon/handler/tool_executor.rs",
    "crates/executive/src/impl/daemon/mcp_embedded.rs",
    "crates/executive/src/impl/runtime/provider_worker.rs",
    "crates/executive/src/service/request_use_cases.rs",
    "crates/executive/src/service/admin_service.rs",
    "crates/executive/src/service/post_turn_projection.rs",
    "crates/executive/src/service/turn_pipeline.rs",
    "crates/executive/src/service/turn_runtime_ports.rs",
]
forbidden = [
    "mnemosyne::FactStore",
    "corpus::tools::tools::ToolRegistry",
    "corpus::HookRegistry",
    "ToolRunnerWithGuard",
    "metacog::r#impl",
    "MorphogenesisPipeline",
    "cognit::harness::linear",
    "LinearCognitiveSession",
    "AletheonExecutive",
]
violations = []
for name in files:
    path = Path(name)
    production = path.read_text().split("#[cfg(test)]", 1)[0]
    for needle in forbidden:
        if needle in production:
            violations.append(f"{name}: {needle}")
if violations:
    raise SystemExit("architecture-check: domain facade bypass:\n" + "\n".join(violations))

request_use_cases = Path("crates/executive/src/service/request_use_cases.rs")
request_source = request_use_cases.read_text().split("#[cfg(test)]", 1)[0]
required_request_ports = [
    "Arc<dyn ExecutiveRuntimePort>",
    "Arc<dyn ReflectionMemoryPort>",
    "Arc<dyn ReflectionEnginePort>",
    "Arc<dyn SelfStatusPort>",
    "Arc<dyn SupplementalMemoryStatusPort>",
    "Arc<dyn RetentionAdminPort>",
]
missing = [port for port in required_request_ports if port not in request_source]
concrete = [
    name
    for name in [
        "AletheonExecutive",
        "EpisodicMemory",
        "SelfField",
        "CompositeMemoryHealth",
        "RetentionRepository",
        "RetentionCompactor",
        "cognit::core::reflector::Reflector",
    ]
    if name in request_source
]
if missing or concrete:
    details = [*(f"missing request port: {port}" for port in missing)]
    details.extend(f"concrete request state: {name}" for name in concrete)
    raise SystemExit(
        "architecture-check: request use-case authority:\n" + "\n".join(details)
    )

turn_runtime = Path("crates/executive/src/service/turn_runtime_ports.rs")
turn_source = turn_runtime.read_text().split("#[cfg(test)]", 1)[0]
required_turn_ports = [
    "Arc<dyn TurnHookPort>",
    "Arc<dyn StormStatePort>",
    "Arc<dyn ModelSelectionPort>",
    "Arc<dyn SelfPolicyPort>",
    "Arc<dyn TurnApprovalPort>",
    "Arc<dyn GovernedTurnCapabilityPort>",
    "Arc<dyn TurnSessionStatePort>",
    "Arc<dyn TurnConfigPort>",
    "Arc<dyn TurnObservabilityPort>",
]
missing = [port for port in required_turn_ports if port not in turn_source]
concrete = [
    name
    for name in [
        "dasein::SelfField",
        "AletheonExecutive",
        "StormBreaker",
        "PendingApproval",
        "CapabilityResources",
        "SessionManager",
        "ModelRouter",
        "PerfCounter",
        "corpus::CorpusService",
        "mnemosyne::MemoryService",
    ]
    if name in turn_source
]
if missing or concrete:
    details = [*(f"missing turn port: {port}" for port in missing)]
    details.extend(f"concrete turn state: {name}" for name in concrete)
    raise SystemExit(
        "architecture-check: turn runtime authority:\n" + "\n".join(details)
    )

exec_session = Path("crates/executive/src/service/exec_session.rs")
exec_source = exec_session.read_text().split("#[cfg(test)]", 1)[0]
if "compose_exec_corpus" not in exec_source:
    raise SystemExit("architecture-check: exec session misses private Corpus composition")
exec_concrete = [
    name
    for name in [
        "ToolRunnerWithGuard",
        "CorpusToolExecutor",
        "DefaultCorpusService",
        "HookRegistry",
        "default_tool_registry",
    ]
    if name in exec_source
]
if exec_concrete:
    raise SystemExit(
        "architecture-check: exec session owns concrete Corpus runtime:\n"
        + "\n".join(exec_concrete)
    )
PY

# M04 deletion gate: recalled memory enters model context only after the
# Mnemosyne projector and C01 selection. Legacy prompt renderers stay deleted.
python3 - <<'PY'
from pathlib import Path

paths = [
    Path("crates/executive/src/service/pre_turn.rs"),
    Path("crates/executive/src/service/context_assembler.rs"),
    Path("crates/executive/src/service/conscious_workspace.rs"),
    Path("crates/executive/src/impl/conscious/memory_processor.rs"),
    Path("crates/executive/src/service/turn_pipeline.rs"),
    Path("crates/executive/src/impl/daemon/prefix_builder.rs"),
]
paths.extend(Path("crates/cognit/src").rglob("*.rs"))
forbidden = [
    "prepare_composite_recall",
    "render_recall_set",
    "RecallInjector",
    "inject_into_prompt",
    "MemoryRequest::FormatForContext",
    "<memory>\\n",
    "mnemosyne::backends",
    "mnemosyne::r#impl",
]
violations = []
for path in paths:
    production = path.read_text().split("#[cfg(test)]", 1)[0]
    for needle in forbidden:
        if needle in production:
            violations.append(f"{path}: {needle}")
memory_adapter = Path("crates/executive/src/impl/conscious/memory_processor.rs").read_text()
if "DefaultMemoryWorkspaceProjector.project" not in memory_adapter:
    violations.append("conscious memory adapter: missing Mnemosyne bounded projector")
if violations:
    raise SystemExit("architecture-check: memory workspace bypass:\n" + "\n".join(violations))
PY

# M05 deletion gate: leased SQLite consolidation is the only exported memory
# pipeline; the former process-local phase/state implementations stay deleted.
python3 - <<'PY'
from pathlib import Path

legacy = Path("crates/mnemosyne/src/impl/pipeline")
if legacy.exists():
    raise SystemExit("architecture-check: legacy in-memory consolidation pipeline exists")
required = [
    "crates/mnemosyne/src/consolidation/repository.rs",
    "crates/mnemosyne/src/consolidation/extractor.rs",
    "crates/mnemosyne/src/consolidation/consolidator.rs",
    "crates/executive/src/service/memory_consolidation_worker.rs",
]
missing = [path for path in required if not Path(path).is_file()]
if missing:
    raise SystemExit("architecture-check: missing durable consolidation boundary: " + ",".join(missing))
service = Path("crates/mnemosyne/src/service.rs").read_text()
if "ScopedConsolidator::new" not in service:
    raise SystemExit("architecture-check: MemoryService bypasses canonical consolidation")
PY

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

# G03/G10 deletion gate: Executive owns the only production AgentControlPort
# implementation. Compatibility runtimes are a registry only and may not own
# lifecycle/run state.
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
spawner_state=$(rg -l '\bSubAgentSpawner\b' crates/executive/src -g '*.rs' || true)
if [[ -n "$spawner_state" ]]; then
  echo "architecture-check: retired SubAgentSpawner run authority remains:" >&2
  echo "$spawner_state" >&2
  exit 1
fi
if rg -n 'struct SubAgentSpawner|HashMap<String, *SubAgentEntry|KernelRuntime|OperationScope|SubAgentHandle' \
  crates/executive/src/core/sub_agent.rs crates/executive/src/core/runtime_registry.rs; then
  echo "architecture-check: compatibility runtime catalog owns Agent run state" >&2
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

# G10 recovery must reconcile durable metadata; it may never call the ordinary
# launch/provider path, which would replay ambiguous work after a crash.
if rg -n '\.launch\(|\.run_in_context\(|provider.*\.complete\(' \
  crates/executive/src/service/agent_control/recovery.rs; then
  echo "architecture-check: Agent recovery replays ordinary runtime/provider work" >&2
  exit 1
fi
if ! rg -q 'reconcile_startup' crates/executive/src/impl/daemon/bootstrap/request.rs; then
  echo "architecture-check: daemon startup skips durable Agent reconciliation" >&2
  exit 1
fi

# G09/M08 deletion gate: child memory authority is process-derived and the
# only broader-scope write is the reviewed promotion module. Agent runtime and
# candidate projection code may not directly mutate root memory or Dasein.
agent_memory_bypass=$(rg -l 'MemoryScope::(Global|Principal|Session)|ApprovedCore|Dasein(Core|Ledger)|\.consolidate\(' \
  crates/executive/src/service/agent_control crates/executive/src/impl/runtime -g '*.rs' \
  | grep -Ev '^crates/executive/src/service/agent_control/memory\.rs$' || true)
if [[ -n "$agent_memory_bypass" ]]; then
  echo "architecture-check: child Agent escaped reviewed memory promotion:" >&2
  echo "$agent_memory_bypass" >&2
  exit 1
fi
if rg -n 'MemoryScope::(Agent|Task)\([^)]*(request|input|argument|scope)' \
  crates/mnemosyne/src/agent_scope.rs crates/executive/src/service/agent_control -g '*.rs'; then
  echo "architecture-check: child Agent scope is derived from caller-provided data" >&2
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
if rg -n '\bAgoraOps\b' \
  crates/executive/src/core/domain_ports.rs \
  crates/executive/src/service/turn_pipeline.rs \
  crates/executive/src/service/exec_session.rs; then
  echo "architecture-check: production composition bypasses authoritative AgoraService" >&2
  exit 1
fi
if rg -n 'pub async fn (publish|update)\(' crates/agora/src/ops/mod.rs; then
  echo "architecture-check: direct Agora mutation API was restored" >&2
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
if (( $(wc -l < crates/executive/src/impl/daemon/bootstrap/request.rs) > 2000 )); then
  echo "architecture-check: bootstrap/request.rs exceeded its composition bound" >&2
  exit 1
fi

if rg -n 'Conservatively no-op|let _ = policy' crates/mnemosyne/src/service.rs; then
  echo "architecture-check: MemoryService forgetting regressed to a silent no-op" >&2
  exit 1
fi
if ! rg -q 'elevated forget requires a matching dry-run preview' crates/mnemosyne/src/retention/repository.rs; then
  echo "architecture-check: elevated memory deletion lost its preview gate" >&2
  exit 1
fi
if rg -n '\.forget_memory\(' crates/executive/src -g '*.rs' \
  | grep -v '^crates/executive/src/service/admin_service.rs:'; then
  echo "architecture-check: governed memory forgetting escaped the admin service" >&2
  exit 1
fi
for stage in channels google runtime storage; do
  if (( $(wc -l < "crates/executive/src/impl/daemon/bootstrap/${stage}.rs") > 700 )); then
    echo "architecture-check: bootstrap/${stage}.rs exceeded its stage bound" >&2
    exit 1
  fi
done
fi

if [[ ${ARCH_SKIP_DEPENDENCIES:-0} != 1 ]]; then
  cargo metadata --no-deps --format-version 1 | python3 -c '
import json,sys
data=json.load(sys.stdin)
names={p["name"] for p in data["packages"]}
reviewed={
    ("aletheon-bin", "fabric"),
    ("corpus", "cognit"),
    ("corpus", "mnemosyne"),
    ("exec-server", "corpus"),
    ("executive", "gateway"),
    ("gateway", "fabric"),
    ("interact", "executive"),
}
for package in data["packages"]:
    for dep in package["dependencies"]:
        if dep["name"] in names and (package["name"], dep["name"]) not in reviewed:
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
