#!/usr/bin/env bash
set -euo pipefail

# Pin C collation so `sort` (line ~158) and `comm` (compare_maximum) agree
# regardless of the caller's locale. The committed baselines under config/ are
# C-sorted; without this, an ambient UTF-8 locale makes `comm` reject the
# C-sorted baseline as "not in sorted order" and abort the gate under `set -e`,
# silently disabling all architecture enforcement.
export LC_ALL=C

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ARCH_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}
ALLOW="$ROOT/config/architecture-allowlist.txt"
DEPS="$ROOT/config/architecture-dependencies.txt"
PATHS="$ROOT/config/architecture-path-inventory.txt"
mkdir -p "$ROOT/target"
actual=$(mktemp)
dep_actual=$(mktemp)
path_actual=$(mktemp)
trap 'rm -f "$actual" "$dep_actual" "$path_actual" "$actual.new" "$actual.stale" "$dep_actual.new" "$dep_actual.stale" "$path_actual.new" "$path_actual.stale"' EXIT
cd "$ROOT"

# Phase 0 architecture inventory and semantic ratchets.  The inventories are
# deliberately data files: later refactor phases update ownership and lower
# metrics without having to rewrite this checker.
if [[ ${ARCH_SKIP_PHASE0_GATES:-0} != 1 && -d config/architecture ]]; then
python3 - <<'PY'
from __future__ import annotations

import os
import re
import sys
import tomllib
from pathlib import Path

root = Path.cwd()
cfg = root / "config/architecture"

def data_lines(name: str):
    path = cfg / name
    if not path.is_file():
        raise SystemExit(f"architecture-check: missing Phase 0 inventory {path}")
    return [line for line in path.read_text().splitlines()
            if line.strip() and not line.startswith("#")]

def production_rs():
    for path in sorted((root / "crates").rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        if "/tests/" in rel or "/examples/" in rel:
            continue
        yield rel, path.read_text(errors="replace").split("#[cfg(test)]", 1)[0]

# A new workspace crate or a new top-level impl tree requires an ownership
# decision rather than silently inheriting a neighbouring crate's policy.
boundary_rows = [line.split("|") for line in data_lines("module-boundaries.txt")
                 if not line.startswith("ownership|")]
recorded_crates = {row[0] for row in boundary_rows}
recorded_impl = {row[0] for row in boundary_rows if len(row) > 4 and row[4] == "true"}
workspace = tomllib.loads((root / "Cargo.toml").read_text()).get("workspace", {})
actual_crates = set()
for member in workspace.get("members", []):
    for manifest in root.glob(f"{member}/Cargo.toml"):
        actual_crates.add(tomllib.loads(manifest.read_text())["package"]["name"])
actual_impl = {p.parent.parent.name for p in (root / "crates").glob("*/src/impl") if p.is_dir()}
if actual_crates != recorded_crates:
    raise SystemExit("architecture-check: workspace crate inventory differs; "
                     f"added={sorted(actual_crates-recorded_crates)}, removed={sorted(recorded_crates-actual_crates)}")
if actual_impl != recorded_impl:
    raise SystemExit("architecture-check: top-level impl inventory differs; "
                     f"added={sorted(actual_impl-recorded_impl)}, removed={sorted(recorded_impl-actual_impl)}")

# Executive files have an explicit target-layer owner.  This makes moves
# reviewable and prevents new modules from silently landing in the legacy tree.
layer_rows = [line.split("\t") for line in data_lines("executive-layers.tsv")]
recorded_layers = {row[0]: row[1] for row in layer_rows}
allowed_layers = {"domain", "application", "adapter", "composition", "host", "compatibility"}
invalid_layers = sorted((path, layer) for path, layer in recorded_layers.items()
                        if layer not in allowed_layers)
if invalid_layers:
    raise SystemExit(f"architecture-check: invalid Executive layer assignments: {invalid_layers}")
actual_executive = {p.relative_to(root).as_posix()
                    for p in (root / "crates/executive/src").rglob("*.rs")}
if actual_executive != set(recorded_layers):
    raise SystemExit("architecture-check: Executive layer inventory differs; "
                     f"added={sorted(actual_executive-set(recorded_layers))}, "
                     f"removed={sorted(set(recorded_layers)-actual_executive)}")

# Every raw AppConfig key has an explicit normalized owner and consumer.
if actual_executive:
    config_rows = [line.split("\t") for line in data_lines("config-ownership.tsv")]
    recorded_config = {row[0] for row in config_rows}
    config_source = (root / "crates/executive/src/composition/config/mod.rs").read_text()
    app_body = config_source.split("pub struct AppConfig {", 1)[1].split("\n}", 1)[0]
    actual_config = set(re.findall(r"^\s*pub\s+(\w+)\s*:", app_body, re.M))
    if actual_config != recorded_config:
        raise SystemExit("architecture-check: AppConfig ownership inventory differs; "
                         f"added={sorted(actual_config-recorded_config)}, "
                         f"removed={sorted(recorded_config-actual_config)}")
# Application code may depend on ports and domain contracts, never host or
# concrete adapters.  The sole exception is a deprecated compatibility shim
# retained for the legacy SQLite repository path until Phase 9.
for rel in sorted(path for path, layer in recorded_layers.items() if layer == "application"):
    body = (root / rel).read_text(errors="replace").split("#[cfg(test)]", 1)[0]
    for lineno, line in enumerate(body.splitlines(), 1):
        if re.search(r"crate::(?:adapters|host)::", line):
            sqlite_shim = (rel == "crates/executive/src/application/agent_control/mod.rs"
                           and "crate::adapters::agent_control::sqlite_repository" in line)
            if not sqlite_shim:
                raise SystemExit(f"architecture-check: Executive application imports a concrete/host layer at {rel}:{lineno}")

# Coding runtime identity and wire types terminate at the private adapter.
# Goal and Agent Control must make policy decisions from neutral request and
# resource contracts rather than a runtime name.
for rel, body in production_rs():
    if not (rel.startswith("crates/executive/src/application/")):
        continue
    if re.search(r"\b(?:PiAttemptRequest|PiRuntime|PI_CODER_RUNTIME_ID)\b|contains\s*\([^\n]*[\"']pi", body, re.I):
        raise SystemExit(f"architecture-check: coding runtime identity leaked into application policy: {rel}")

# Channel/source application code is provider-neutral. Concrete provider
# vocabulary is confined to Gateway adapters, Corpus adapters, Executive
# adapters/host compatibility, and the composition factory entry point.
for rel, body in production_rs():
    provider_leak = re.search(r"\b(?:Google|Gmail|Telegram)(?:[A-Z]\w*)?\b|\b(?:google|gmail|telegram)_", body)
    if rel.startswith("crates/executive/src/application/") and provider_leak:
        raise SystemExit(f"architecture-check: provider identity leaked into Executive application: {rel}")
    if (rel.startswith("crates/gateway/src/") and
            not rel.startswith("crates/gateway/src/adapters/") and
            rel != "crates/gateway/src/lib.rs" and provider_leak):
        raise SystemExit(f"architecture-check: provider identity leaked into Gateway core: {rel}")

factory_path = root / "crates/cognit/src/composition/inference_factory.rs"
if factory_path.is_file():
    factory_source = factory_path.read_text()
    if "detect_compatibility_kind" in factory_source or re.search(
            r"Transport::Auto[^\n]*(?:base_url|endpoint)|(?:base_url|endpoint)[^\n]*Transport::Auto",
            factory_source):
        raise SystemExit("architecture-check: inference factory restored URL-based provider guessing")

# Every explicit protocol and migration file has an owner before it can land.
wire_paths = {line.split("\t")[2] for line in data_lines("wire-surfaces.tsv")}
wire_candidates = set()
for path in (root / "crates").rglob("*.proto"):
    wire_candidates.add(path.relative_to(root).as_posix())
for rel in ("crates/execd/src/protocol.rs", "crates/executive/src/host/core_rpc/protocol.rs"):
    if (root / rel).is_file(): wire_candidates.add(rel)
for path in (root / "crates/fabric/src/protocol").glob("*.rs"):
    wire_candidates.add(path.relative_to(root).as_posix())
missing_wire = sorted(wire_candidates - wire_paths)
if missing_wire:
    raise SystemExit("architecture-check: unregistered wire surface: " + ", ".join(missing_wire))

persistence_paths = {line.split("\t")[2] for line in data_lines("persistence-surfaces.tsv")}
migration_candidates = set()
for path in (root / "crates").rglob("*"):
    if not path.is_file(): continue
    rel = path.relative_to(root).as_posix()
    if ("/migrations/" in rel and path.suffix in {".sql", ".rs"}) or path.name in {"migration.rs", "migrations.rs"}:
        migration_candidates.add(rel)
missing_persistence = sorted(migration_candidates - persistence_paths)
if missing_persistence:
    raise SystemExit("architecture-check: unregistered persistence migration: " + ", ".join(missing_persistence))

sources = list(production_rs())
core_prefixes = ("crates/fabric/", "crates/kernel/", "crates/executive/src/application/",
                 "crates/cognit/src/harness/")
def core(rel):
    return rel.startswith(core_prefixes) or any(part in rel.split("/") for part in ("domain", "contract", "application"))

identifier_rules = []
for line in data_lines("external-identifiers.txt"):
    name, pattern, allowed, *_ = line.split("\t")
    identifier_rules.append((name, re.compile(pattern), tuple(x for x in allowed.split(",") if x != "-")))

metrics = {
    "CORE_EXTERNAL_IDENTIFIER_HITS": 0,
    "PUBLIC_IMPL_ADAPTER_EXPORTS": 0,
    "CROSS_CRATE_IMPL_REFERENCES": 0,
    "PROVIDER_NAME_BRANCHES": 0,
    "URL_PROVIDER_INFERENCE": 0,
    "PROVIDER_ERROR_TEXT_BRANCHES": 0,
    "FORBIDDEN_INFRA_IMPORTS": 0,
    "CORE_OPAQUE_VALUE_INSPECTIONS": 0,
    "FABRIC_PROVIDER_TYPES": 0,
}
for rel, body in sources:
    lines = body.splitlines()
    if core(rel):
        for line in lines:
            if any(rx.search(line) and not any(rel.startswith(p) for p in allowed)
                   for _, rx, allowed in identifier_rules):
                metrics["CORE_EXTERNAL_IDENTIFIER_HITS"] += 1
    metrics["PUBLIC_IMPL_ADAPTER_EXPORTS"] += sum(bool(re.search(r"\bpub\s+(?:mod|use)\s+(?:r#impl|impl|adapter|adapters)\b", line)) for line in lines)
    if rel.startswith("crates/fabric/"):
        metrics["FABRIC_PROVIDER_TYPES"] += sum(bool(re.search(r"\b(?:pub\s+)?(?:struct|enum|trait|type)\s+\w*(?:Google|Gmail|Drive|Anthropic|OpenAi|Ollama|Telegram)\w*", line)) for line in lines)
    metrics["CROSS_CRATE_IMPL_REFERENCES"] += sum(bool(re.search(r"\b(?:agora|cognit|corpus|dasein|executive|gateway|hardware|metacog|mnemosyne)::(?:r#impl|impl)::", line)) for line in lines)
    if (core(rel) and not any(part in rel for part in
            ("/adapter/", "/adapters/", "/impl/", "/composition/", "/factory", "/registry"))):
        metrics["PROVIDER_NAME_BRANCHES"] += sum(bool(re.search(r"(?:match\s+\w*provider|contains\s*\([^)]*provider|provider[^\n]*(?:==|!=)\s*\")", line, re.I)) for line in lines)
        metrics["URL_PROVIDER_INFERENCE"] += sum(bool(re.search(r"(?:url|endpoint).*(?:contains|starts_with).*(?:provider|anthropic|openai|ollama)", line, re.I)) for line in lines)
        metrics["PROVIDER_ERROR_TEXT_BRANCHES"] += sum(bool(re.search(r"(?:error|message).*(?:contains|starts_with).*(?:provider|anthropic|openai|ollama)", line, re.I)) for line in lines)
        metrics["FORBIDDEN_INFRA_IMPORTS"] += sum(bool(re.search(r"(?:use\s+[^;]*(?:::adapter|::r#impl|::impl)|\b(?:sqlx|reqwest|tonic)::)", line)) for line in lines)
        metrics["CORE_OPAQUE_VALUE_INSPECTIONS"] += sum(bool(re.search(r"(?:serde_json::Value|\bValue\b).*(?:\.get\(|\[\s*\")|(?:\.get\(|\[\s*\").*(?:serde_json::Value|\bValue\b)", line)) for line in lines)

if os.environ.get("ARCH_PRINT_PHASE0_METRICS") == "1":
    for key in sorted(metrics): print(f"{key}={metrics[key]}")
    sys.exit(0)

baseline = {}
for line in data_lines("metrics.env"):
    key, value = line.split("=", 1); baseline[key] = int(value)
if set(metrics) != set(baseline):
    raise SystemExit(f"architecture-check: metrics inventory keys differ; current={sorted(metrics)}, baseline={sorted(baseline)}")
changed = [(key, baseline[key], metrics[key]) for key in sorted(metrics) if baseline[key] != metrics[key]]
if changed:
    detail = ", ".join(f"{key}:{old}->{new}" for key, old, new in changed)
    raise SystemExit("architecture-check: Phase 0 metric changed; update/lower the reviewed baseline: " + detail)

# Compatibility exceptions are individually counted and ratcheted.
for row in data_lines("compatibility-debt.tsv"):
    debt_id, rel, pattern, _, _, expected, _ = row.split("\t")
    path = root / rel
    actual = len(re.findall(pattern, path.read_text(errors="replace"))) if path.is_file() else 0
    if actual != int(expected):
        raise SystemExit(f"architecture-check: compatibility debt {debt_id} changed {expected}->{actual}; update/lower its reviewed baseline")
PY
fi

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
  crates/executive/src crates/aletheon/src -g '*.rs' \
  | grep -v '^crates/executive/src/application/extension_service.rs$' || true)
if [[ -n "$extension_activation_outside_owner" ]]; then
  echo "architecture-check: extension activation bypasses Executive ExtensionService:" >&2
  echo "$extension_activation_outside_owner" >&2
  exit 1
fi
if [[ ! -s config/schema/aletheon-config.schema.json ]]; then
  echo "architecture-check: checked-in application config schema is missing" >&2
  exit 1
fi

# H2: provider identity and construction have one owner. Heuristic inference
# candidates must not recreate the application provider schema, and every
# concrete LLM constructor must remain behind the canonical factory.
provider_config_defs=$(rg -n '\bstruct ProviderConfig\b' crates/cognit/src -g '*.rs' | wc -l)
if [[ "$provider_config_defs" -ne 1 ]]; then
  echo "architecture-check: Cognit must define exactly one ProviderConfig (found $provider_config_defs)" >&2
  exit 1
fi
provider_constructors_outside_factory=$(rg -l \
  'AnthropicProvider::new|OpenAiProvider::new|OllamaProvider::new' \
  crates/cognit/src -g '*.rs' \
  | grep -v '^crates/cognit/src/composition/inference_factory.rs$' || true)
if [[ -n "$provider_constructors_outside_factory" ]]; then
  echo "architecture-check: concrete LLM construction bypasses provider_factory:" >&2
  echo "$provider_constructors_outside_factory" >&2
  exit 1
fi

# H3: legacy business environment parsing is a compatibility concern owned by
# Executive's typed config loader. Host protocol variables (systemd, XDG,
# display, credential directory, sockets, and subprocess handoff) are excluded.
h3_business_env_reads=$(rg -n \
  'std::env::(?:var|var_os)\("(?:AGENT_(?:WORKING_DIR|DATA_DIR|SYSTEM_PROMPT|SANDBOX_PREFERENCE)|ALETHEON_CONSCIOUS_ARBITRATION_MODE|ALETHEON_GOOGLE_(?:CLIENT_ID|CLIENT_SECRET|REDIRECT_URI|DRIVE_SYNC_ENABLED|DRIVE_FILE_IDS)|ALETHEON_GMAIL_INGRESS_POLICY_FILE|SEARCH_API_(?:URL|KEY))"' \
  crates -g '*.rs' \
  | grep -v '^crates/executive/src/composition/config/' || true)
if [[ -n "$h3_business_env_reads" ]]; then
  echo "architecture-check: business environment parsing bypasses typed bootstrap config:" >&2
  echo "$h3_business_env_reads" >&2
  exit 1
fi

# H4: MCP production background work must be registered with the MCP task
# supervisor. Test-only mock servers remain outside this client gate.
if rg -n 'tokio::spawn' crates/corpus/src/tools/mcp/client.rs; then
  echo "architecture-check: MCP client background task bypasses McpTaskSupervisor" >&2
  exit 1
fi

# Q02 deletion gates: Interact and Bin may depend on Fabric protocol types, while
# domain construction belongs to Executive/Corpus composition.
if rg -n '^\s*(kernel|corpus)\s*=' crates/interact/Cargo.toml || \
   rg -n '\b(kernel|corpus)::|use\s+(kernel|corpus)\b' \
     crates/interact/src -g '*.rs'; then
  echo "architecture-check: Interact imports Kernel or Corpus" >&2
  exit 1
fi
if rg -n '\b(kernel|corpus|cognit|mnemosyne|dasein|agora|metacog)\s*=' \
     crates/aletheon/Cargo.toml || \
   rg -n '\b(ExecSessionBuilder|TurnRequest|RuntimeHost|KernelRuntime|ToolRegistry)\b|\b(corpus|cognit|mnemosyne|dasein|agora|metacog)::' \
     crates/aletheon/src -g '*.rs'; then
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
  crates/aletheon/src/lib.rs; do
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
for root in (Path("crates/corpus"), Path("crates/executive"), Path("crates/aletheon")):
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
scan core_systems_field '\.(runtime|domain|infra|orchestration|memory)\.' crates/executive/src crates/aletheon/src \
  -g '!**/application/admin_service.rs' -g '!**/application/post_turn_projection.rs'
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
        "/host/daemon/bootstrap/" in name
        or name == "crates/executive/src/composition/exec_corpus.rs"
        or name == "crates/executive/src/application/conscious_workspace.rs"
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
    "crates/executive/src/host/daemon/handler/mod.rs",
    "crates/executive/src/host/daemon/handler/init.rs",
    "crates/executive/src/host/daemon/handler/ports.rs",
    "crates/executive/src/host/daemon/handler/tool_executor.rs",
    "crates/executive/src/host/daemon/mcp_embedded.rs",
    "crates/executive/src/adapters/runtime/provider_worker.rs",
    "crates/executive/src/application/request_use_cases.rs",
    "crates/executive/src/application/admin_service.rs",
    "crates/executive/src/application/post_turn_projection.rs",
    "crates/executive/src/application/turn_pipeline.rs",
    "crates/executive/src/application/turn_runtime_ports.rs",
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

request_use_cases = Path("crates/executive/src/application/request_use_cases.rs")
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

turn_runtime = Path("crates/executive/src/application/turn_runtime_ports.rs")
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

exec_session = Path("crates/executive/src/composition/exec_session.rs")
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
    Path("crates/executive/src/application/pre_turn.rs"),
    Path("crates/executive/src/application/context_assembler.rs"),
    Path("crates/executive/src/application/conscious_workspace.rs"),
    Path("crates/executive/src/application/conscious/memory_processor.rs"),
    Path("crates/executive/src/application/turn_pipeline.rs"),
    Path("crates/executive/src/composition/prefix_builder.rs"),
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
memory_adapter = Path("crates/executive/src/application/conscious/memory_processor.rs").read_text()
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
    "crates/executive/src/application/memory_consolidation_worker.rs",
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
  crates/executive/src crates/aletheon/src; then
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
if [[ "$agent_control_impls" != "crates/executive/src/application/agent_control/mod.rs" ]]; then
  echo "architecture-check: AgentControlPort has a non-authoritative implementation:" >&2
  echo "$agent_control_impls" >&2
  exit 1
fi
if rg -n '\bSubAgentSpawner\b' crates/corpus/src -g '*.rs'; then
  echo "architecture-check: Corpus bypasses AgentControlPort through SubAgentSpawner" >&2
  exit 1
fi
if rg -n '\bExecuteSubAgentFn\b' crates/corpus/src crates/executive/src/host/daemon/bootstrap \
  crates/executive/src/application/agent_control -g '*.rs'; then
  echo "architecture-check: Agent execution closure bypasses AgentControlPort" >&2
  exit 1
fi
if rg -n '\.complete\(' crates/executive/src/host/daemon/bootstrap \
  crates/executive/src/application/agent_control -g '*.rs'; then
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
  crates/executive/src/application/agent_control/candidate_projection.rs \
  crates/executive/src/adapters/runtime/native_cognit.rs; then
  echo "architecture-check: child Agent bypasses C01 candidate admission" >&2
  exit 1
fi

# G07 deletion gate: Kernel owns the registry and AgentControl owns the only
# application-level live Agent mailbox registration adapter.
mailbox_registration_outside_owner=$(rg -l 'register_process_mailbox' crates -g '*.rs' -g '!**/tests/**' \
  | grep -Ev '^crates/(kernel/src/runtime\.rs|executive/src/application/agent_control/mod\.rs)$' || true)
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
if rg -n 'BoundedAgentAdmission::new\(' crates/executive/src/host/daemon/bootstrap -g '*.rs'; then
  echo "architecture-check: production Agent admission bypasses typed Kernel-backed config" >&2
  exit 1
fi

# G10 recovery must reconcile durable metadata; it may never call the ordinary
# launch/provider path, which would replay ambiguous work after a crash.
if rg -n '\.launch\(|\.run_in_context\(|provider.*\.complete\(' \
  crates/executive/src/application/agent_control/recovery.rs; then
  echo "architecture-check: Agent recovery replays ordinary runtime/provider work" >&2
  exit 1
fi
if ! rg -q 'reconcile_startup' crates/executive/src/host/daemon/bootstrap/request.rs crates/executive/src/host/daemon/bootstrap/services.rs; then
  echo "architecture-check: daemon startup skips durable Agent reconciliation" >&2
  exit 1
fi

# G09/M08 deletion gate: child memory authority is process-derived and the
# only broader-scope write is the reviewed promotion module. Agent runtime and
# candidate projection code may not directly mutate root memory or Dasein.
agent_memory_bypass=$(rg -l 'MemoryScope::(Global|Principal|Session)|ApprovedCore|Dasein(Core|Ledger)|\.consolidate\(' \
  crates/executive/src/application/agent_control crates/executive/src/adapters/runtime -g '*.rs' \
  | grep -Ev '^crates/executive/src/application/agent_control/memory\.rs$' || true)
if [[ -n "$agent_memory_bypass" ]]; then
  echo "architecture-check: child Agent escaped reviewed memory promotion:" >&2
  echo "$agent_memory_bypass" >&2
  exit 1
fi
if rg -n 'MemoryScope::(Agent|Task)\([^)]*(request|input|argument|scope)' \
  crates/mnemosyne/src/agent_scope.rs crates/executive/src/application/agent_control -g '*.rs'; then
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
   rg -n '\bCoreSystems\b|\.subsystems\b' crates/executive/src crates/aletheon/src -g '*.rs'; then
  echo "architecture-check: retired god container escaped into production" >&2
  exit 1
fi
if rg -n '\bAgoraOps\b' \
  crates/executive/src/core/domain_ports.rs \
  crates/executive/src/application/turn_pipeline.rs \
  crates/executive/src/composition/exec_session.rs; then
  echo "architecture-check: production composition bypasses authoritative AgoraService" >&2
  exit 1
fi
if rg -n 'pub async fn (publish|update)\(' crates/agora/src/ops/mod.rs; then
  echo "architecture-check: direct Agora mutation API was restored" >&2
  exit 1
fi
composition_outside_bootstrap=$(rg -l '\bDaemonComposition\b' crates/executive/src -g '*.rs' \
  | grep -v '^crates/executive/src/host/daemon/bootstrap/' || true)
if [[ -n "$composition_outside_bootstrap" ]]; then
  echo "architecture-check: private daemon composition escaped bootstrap:" >&2
  echo "$composition_outside_bootstrap" >&2
  exit 1
fi
if (( $(wc -l < crates/executive/src/host/daemon/handler/init.rs) > 250 )); then
  echo "architecture-check: handler/init.rs is no longer a thin compatibility layer" >&2
  exit 1
fi
if (( $(wc -l < crates/executive/src/host/daemon/bootstrap/request.rs) > 2000 )); then
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
  | grep -v '^crates/executive/src/application/admin_service.rs:'; then
  echo "architecture-check: governed memory forgetting escaped the admin service" >&2
  exit 1
fi
for stage in channels google runtime storage; do
  if (( $(wc -l < "crates/executive/src/host/daemon/bootstrap/${stage}.rs") > 700 )); then
    echo "architecture-check: bootstrap/${stage}.rs exceeded its stage bound" >&2
    exit 1
  fi
done
fi

if [[ ${ARCH_SKIP_DEPENDENCIES:-0} != 1 ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "architecture-check: cargo not found; dependency graph gate cannot run." >&2
    echo "  Install Rust/cargo, or set ARCH_SKIP_DEPENDENCIES=1 to explicitly skip." >&2
    exit 1
  fi
  bash "$SCRIPT_DIR/cargo-agent.sh" metadata --no-deps --format-version 1 | python3 -c '
import json,sys
data=json.load(sys.stdin)
names={p["name"] for p in data["packages"]}
forbidden=sorted(name for name in names if "-" in name)
if forbidden:
    raise SystemExit("architecture-check: forbidden hyphenated workspace package(s): " + ", ".join(forbidden))
reviewed={
    ("aletheon", "fabric"),
    ("cognit", "kernel"),
    ("executive", "gateway"),
    ("executive", "hardware"),
    ("gateway", "fabric"),
    ("hardware", "fabric"),
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

# Freeze: fabric root-level re-exports must not grow beyond the ledgered
# baseline (architecture-status.toml [freeze].fabric_root_reexports_max).
fabric_reexports_now=$(grep -c '^pub use' crates/fabric/src/lib.rs || echo 0)
fabric_reexports_max=$(grep -E '^\s*fabric_root_reexports_max\s*=' architecture-status.toml \
  | grep -oE '[0-9]+' | head -1)
if [[ -z "$fabric_reexports_max" ]]; then
  echo "architecture-check: architecture-status.toml missing fabric_root_reexports_max" >&2
  exit 1
fi
if (( fabric_reexports_now > fabric_reexports_max )); then
  echo "architecture-check: fabric root re-exports grew from ${fabric_reexports_max} to ${fabric_reexports_now}" >&2
  echo "  New root-level 'pub use' in crates/fabric/src/lib.rs are frozen (Wave 0)." >&2
  echo "  Import from a submodule, or lower the baseline as re-exports are removed." >&2
  exit 1
fi

# Migration path inventory is symbol based and intentionally stable across line moves.
{
  rg -l 'pub struct TurnService' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#turn_path|#; s#$#|TurnService#' || true
  rg -l 'pub struct TurnPipeline' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#turn_path|#; s#$#|TurnPipeline#' || true
  rg -l 'impl TurnServices for ExecTurnServices' crates/executive/src -g '*.rs' 2>/dev/null | sed 's#^#capability_path|#; s#$#|ExecTurnServices#' || true
  rg -l 'CapabilityInvoker for' crates -g '*.rs' -g '!**/tests/**' 2>/dev/null \
    | grep -v 'crates/executive/src/application/governed_capability.rs' \
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
