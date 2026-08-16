#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/config/architecture" "$tmp/target" \
  "$tmp/crates/corpus/src/legacy" "$tmp/crates/dasein/src" \
  "$tmp/crates/executive/src" "$tmp/crates/interact/src" "$tmp/crates/contracts/src"
mkdir -p "$tmp/crates/metacog/src"
# The production architecture gate requires the reviewed retired-authority
# inventory even for synthetic fixtures. Copy the repository baseline so this
# test exercises the fixture-specific findings rather than failing at setup.
cp "$ROOT/config/architecture/retired-authorities.tsv" \
  "$tmp/config/architecture/retired-authorities.tsv"
printf 'pub use example::Example;\n' > "$tmp/crates/contracts/src/lib.rs"
cat > "$tmp/architecture-status.toml" <<'TOML'
[freeze]
fabric_root_reexports_max = 1
TOML
cat > "$tmp/crates/corpus/src/legacy/mod.rs" <<'RS'
tool.execute(x)
RS
cat > "$tmp/crates/dasein/src/lib.rs" <<'RS'
fn clock() { SystemClock::new(); }
RS
cat > "$tmp/crates/executive/src/lib.rs" <<'RS'
use contracts::envelope::Envelope;
use executive::impl::kernel::Table;
fn fields(x: X) { let _ = x.runtime; }
RS
cat > "$tmp/config/architecture-allowlist.txt" <<'BASE'
concrete_clock|crates/dasein/src/lib.rs|fn clock() { SystemClock::new(); }
core_systems_field|crates/executive/src/lib.rs|fn fields(x: X) { let _ = x.runtime; }
direct_tool|crates/corpus/src/legacy/mod.rs|tool.execute(x)
duplicate_kernel|crates/executive/src/lib.rs|use executive::impl::kernel::Table;
executive_store_import|crates/executive/src/application/verification/command.rs|use corpus::tools::subagent::CommandRunner;
legacy_event|crates/executive/src/lib.rs|use contracts::envelope::Envelope;
BASE
: > "$tmp/config/architecture-dependencies.txt"
: > "$tmp/config/architecture-path-inventory.txt"
ARCH_ROOT="$tmp" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null

# A refactor may relocate an already-ledgered finding, but it must not increase
# the number of findings in that category relative to the PR base.
git -C "$tmp" init -q
git -C "$tmp" config user.name architecture-fixture
git -C "$tmp" config user.email architecture-fixture@example.invalid
git -C "$tmp" add config
git -C "$tmp" commit -qm baseline
# Renaming Executive to Aletheon relocates the same ledgered application debt;
# it must retain the old category budget rather than appear as new debt.
mkdir -p "$tmp/crates/aletheon/src/wiring/application/verification"
printf 'use corpus::tools::subagent::CommandRunner;\n' \
  > "$tmp/crates/aletheon/src/wiring/application/verification/command.rs"
sed -i \
  's#executive_store_import|crates/executive/src/application/verification/command.rs#application_store_import|crates/aletheon/src/wiring/application/verification/command.rs#' \
  "$tmp/config/architecture-allowlist.txt"
sort -o "$tmp/config/architecture-allowlist.txt" "$tmp/config/architecture-allowlist.txt"
mv "$tmp/crates/corpus/src/legacy/mod.rs" "$tmp/crates/corpus/src/legacy/moved.rs"
sed -i 's#legacy/mod.rs#legacy/moved.rs#' "$tmp/config/architecture-allowlist.txt"
ARCH_ROOT="$tmp" ARCH_BASE_REF=HEAD ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  ARCH_SKIP_X1_GATES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null
rm -r "$tmp/crates/aletheon"
printf 'tool.execute(y)\n' >> "$tmp/crates/corpus/src/legacy/moved.rs"
printf 'direct_tool|crates/corpus/src/legacy/moved.rs|tool.execute(y)\n' \
  >> "$tmp/config/architecture-allowlist.txt"
sort -o "$tmp/config/architecture-allowlist.txt" "$tmp/config/architecture-allowlist.txt"
if ARCH_ROOT="$tmp" ARCH_BASE_REF=HEAD ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected architecture category growth to fail' >&2; exit 1
fi
sed -i '/tool.execute(y)/d' "$tmp/crates/corpus/src/legacy/moved.rs"
sed -i '/tool.execute(y)/d' "$tmp/config/architecture-allowlist.txt"

mkdir -p "$tmp/crates/metacog/src/core"
if ARCH_ROOT="$tmp" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected retired Metacog roots to fail' >&2; exit 1
fi
rmdir "$tmp/crates/metacog/src/core"
printf 'tool.execute(y)\n' >> "$tmp/crates/corpus/src/legacy/moved.rs"
if ARCH_ROOT="$tmp" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a new finding to fail' >&2; exit 1
fi
sed -i '$d' "$tmp/crates/corpus/src/legacy/moved.rs"
rm "$tmp/crates/dasein/src/lib.rs"
out=$(ARCH_ROOT="$tmp" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh")
grep -q 'resolved findings entries' <<<"$out"

# A local dependency edge not present in the maximum baseline must also fail.
deps="$tmp/deps"; mkdir -p "$deps/config" "$deps/target" "$deps/crates/contracts/src" "$deps/crates/kernel/src"
cat > "$deps/Cargo.toml" <<'TOML'
[workspace]
resolver = "2"
members = ["crates/contracts", "crates/kernel"]
TOML
printf 'pub fn fabric() {}\n' > "$deps/crates/contracts/src/lib.rs"
printf 'pub fn kernel() {}\n' > "$deps/crates/kernel/src/lib.rs"
cat > "$deps/crates/contracts/Cargo.toml" <<'TOML'
[package]
name = "contracts"
version = "0.1.0"
edition = "2021"
[dependencies]
kernel = { path = "../kernel" }
TOML
cat > "$deps/crates/kernel/Cargo.toml" <<'TOML'
[package]
name = "kernel"
version = "0.1.0"
edition = "2021"
TOML
: > "$deps/config/architecture-allowlist.txt"
: > "$deps/config/architecture-dependencies.txt"
: > "$deps/config/architecture-path-inventory.txt"
if ARCH_ROOT="$deps" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a new dependency to fail' >&2; exit 1
fi

# Workspace package names are semantic domain names. Do not permit the deleted
# api/types/broker/platform-* split-crate convention (or any other hyphenated
# package name) to return.
sed -i '/^\[dependencies\]/,$d' "$deps/crates/contracts/Cargo.toml"
sed -i 's/name = "kernel"/name = "runtime-api"/' "$deps/crates/kernel/Cargo.toml"
if ARCH_ROOT="$deps" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 \
  bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a hyphenated workspace package to fail' >&2; exit 1
fi

# Phase 0 semantic gates are fixture-driven.  The clean fixture deliberately
# contains the two legal exceptions: adapter-id selection in composition and
# opaque JSON inspection inside an adapter.
phase0="$tmp/phase0"
mkdir -p "$phase0/config/architecture" "$phase0/config" "$phase0/target" \
  "$phase0/crates/contracts/src/protocol" "$phase0/crates/contracts/src/application" \
  "$phase0/crates/contracts/src/composition" "$phase0/crates/contracts/src/adapter"
cp "$ROOT/config/architecture/retired-authorities.tsv" \
  "$phase0/config/architecture/retired-authorities.tsv"
cat > "$phase0/Cargo.toml" <<'TOML'
[workspace]
resolver = "2"
members = ["crates/contracts"]
TOML
cat > "$phase0/crates/contracts/Cargo.toml" <<'TOML'
[package]
name = "contracts"
version = "0.1.0"
edition = "2021"
TOML
cat > "$phase0/crates/contracts/src/lib.rs" <<'RS'
pub fn stable_contract() {}
RS
printf 'pub struct Request;\n' > "$phase0/crates/contracts/src/protocol/client.rs"
cat > "$phase0/crates/contracts/src/composition/registry.rs" <<'RS'
fn construct(adapter_id: &str) { match adapter_id { "messages-http" => (), _ => () } }
RS
cat > "$phase0/crates/contracts/src/adapter/json.rs" <<'RS'
fn decode(value: serde_json::Value) { let _ = value.get("open_payload"); }
RS
cat > "$phase0/architecture-status.toml" <<'TOML'
[freeze]
fabric_root_reexports_max = 0
TOML
: > "$phase0/config/architecture-allowlist.txt"
: > "$phase0/config/architecture-dependencies.txt"
: > "$phase0/config/architecture-path-inventory.txt"
cat > "$phase0/config/architecture/module-boundaries.txt" <<'EOF'
# frozen_commit=fixture
contracts|crates/contracts|-|protocol|false|adapter
EOF
cat > "$phase0/config/architecture/executive-layers.tsv" <<'EOF'
# frozen_commit=fixture
# fixture has no Executive crate
EOF
cat > "$phase0/config/architecture/external-identifiers.txt" <<'EOF'
# frozen_commit=fixture
evil	\bEvilCorp\b	crates/contracts/src/adapter/	fixture external name	neutral contract	1
EOF
cat > "$phase0/config/architecture/wire-surfaces.tsv" <<'EOF'
# frozen_commit=fixture
wire-exposed	Request	crates/contracts/src/protocol/client.rs	client,server	fabric	v1	additive	1
EOF
cat > "$phase0/config/architecture/persistence-surfaces.tsv" <<'EOF'
# frozen_commit=fixture
fixture	fabric	crates/contracts/src/store.rs	v1	reader	writer	versioned	1
EOF
cat > "$phase0/config/architecture/compatibility-debt.tsv" <<'EOF'
# frozen_commit=fixture
legacy	crates/contracts/src/lib.rs	LEGACY	fixture debt	stable contract	0	1
EOF
cat > "$phase0/config/architecture/metrics.env" <<'EOF'
# frozen_commit=fixture
CORE_EXTERNAL_IDENTIFIER_HITS=0
CORE_OPAQUE_VALUE_INSPECTIONS=0
CROSS_CRATE_IMPL_REFERENCES=0
FABRIC_PROVIDER_TYPES=0
FORBIDDEN_INFRA_IMPORTS=0
FORBIDDEN_DEPENDENCY_EDGES=0
PRODUCTION_CLI_PARSERS=0
PROVIDER_ERROR_TEXT_BRANCHES=0
PROVIDER_NAME_BRANCHES=0
PUBLIC_IMPL_ADAPTER_EXPORTS=0
SESSION_APPEND_WRITERS=0
URL_PROVIDER_INFERENCE=0
EOF
phase0_check() {
  ARCH_ROOT="$phase0" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 ARCH_SKIP_X1_GATES=1 \
    bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1
}
phase0_check || {
  echo 'expected clean Phase 0 fixture to pass' >&2
  ARCH_ROOT="$phase0" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 ARCH_SKIP_X1_GATES=1 \
    bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh"
  exit 1
}
expect_phase0_rejection() {
  if phase0_check; then echo "expected Phase 0 gate to reject $1" >&2; exit 1; fi
}
mkdir -p "$phase0/crates/extra/src"; printf 'pub fn extra() {}\n' > "$phase0/crates/extra/src/lib.rs"
cat > "$phase0/crates/extra/Cargo.toml" <<'TOML'
[package]
name = "extra"
version = "0.1.0"
edition = "2021"
TOML
sed -i 's#members = \["crates/contracts"\]#members = ["crates/contracts", "crates/extra"]#' "$phase0/Cargo.toml"
expect_phase0_rejection 'unregistered workspace crate'
sed -i 's#members = \["crates/contracts", "crates/extra"\]#members = ["crates/contracts"]#' "$phase0/Cargo.toml"; rm -r "$phase0/crates/extra"
mkdir -p "$phase0/crates/contracts/src/impl"
expect_phase0_rejection 'unregistered top-level impl tree'; rmdir "$phase0/crates/contracts/src/impl"
printf 'fn leak() { let _ = EvilCorp::new(); }\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'external name in core'; rm "$phase0/crates/contracts/src/application/leak.rs"
printf 'use executive::adapter::Store;\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'application adapter import'; rm "$phase0/crates/contracts/src/application/leak.rs"
printf 'pub mod adapter;\n' >> "$phase0/crates/contracts/src/lib.rs"
expect_phase0_rejection 'public adapter export'; sed -i '$d' "$phase0/crates/contracts/src/lib.rs"
printf 'fn choose(provider: &str) { if provider == "evil" {} }\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'provider-name business branch'; rm "$phase0/crates/contracts/src/application/leak.rs"
printf 'pub struct NewWire;\n' > "$phase0/crates/contracts/src/protocol/new_wire.rs"
expect_phase0_rejection 'unregistered wire surface'; rm "$phase0/crates/contracts/src/protocol/new_wire.rs"
mkdir -p "$phase0/crates/contracts/src/migrations"; printf 'SELECT 1;\n' > "$phase0/crates/contracts/src/migrations/001.sql"
expect_phase0_rejection 'unregistered persistence migration'; rm -r "$phase0/crates/contracts/src/migrations"
printf '// LEGACY\n' >> "$phase0/crates/contracts/src/lib.rs"
expect_phase0_rejection 'compatibility debt growth'; sed -i '$d' "$phase0/crates/contracts/src/lib.rs"
printf 'fn inspect(value: serde_json::Value) { let _ = value.get("business_kind"); }\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'opaque JSON field inspection in core'; rm "$phase0/crates/contracts/src/application/leak.rs"
printf 'fn main() { Cli::parse(); }\n' > "$phase0/crates/contracts/src/main.rs"
expect_phase0_rejection 'additional production CLI parser'; rm "$phase0/crates/contracts/src/main.rs"
printf 'impl SessionAppendStore for Rogue {}\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'additional Session append writer'; rm "$phase0/crates/contracts/src/application/leak.rs"
printf 'fn bypass() { hardware::grpc::provider::connect(); }\n' > "$phase0/crates/contracts/src/application/leak.rs"
expect_phase0_rejection 'hardware adapter bypass'; rm "$phase0/crates/contracts/src/application/leak.rs"
mkdir -p "$phase0/crates/interact"
cat > "$phase0/crates/interact/Cargo.toml" <<'TOML'
[package]
name = "interact"
version = "0.1.0"
edition = "2021"
[dependencies]
corpus = { path = "../corpus" }
TOML
expect_phase0_rejection 'interact to corpus forbidden dependency'; rm -r "$phase0/crates/interact"
phase0_check || { echo 'legal composition/adapter exceptions regressed' >&2; exit 1; }
echo 'Phase 0 architecture fixtures: pass'
echo 'architecture-check fixture: pass'

# X1 contract registries have their own minimal fixture so public Fabric type
# metadata, acceptance bindings, duplicate ID wrappers and duplicate RPC arms
# are proven fail-closed rather than inferred from the production checkout.
x1="$tmp/x1"
mkdir -p "$x1/config/architecture" "$x1/config" "$x1/target" \
  "$x1/crates/contracts/src" "$x1/crates/contracts/tests" "$x1/docs" \
  "$x1/crates/aletheon/src/wiring/daemon/handler"
cp "$ROOT/config/architecture/retired-authorities.tsv" \
  "$x1/config/architecture/retired-authorities.tsv"
: > "$x1/config/architecture-allowlist.txt"
: > "$x1/config/architecture-dependencies.txt"
: > "$x1/config/architecture-path-inventory.txt"
cat > "$x1/architecture-status.toml" <<'TOML'
[freeze]
fabric_root_reexports_max = 0
TOML
printf 'pub struct Baseline;\n' > "$x1/crates/contracts/src/lib.rs"
printf '# decision\n' > "$x1/docs/decision.md"
cat > "$x1/crates/contracts/tests/architecture_contract.rs" <<'RS'
fn a_dep_001_fixture() {}
fn a_dep_002_fixture() {}
fn a_dep_003_fixture() {}
RS
cat > "$x1/config/architecture/contract-migrations.tsv" <<'EOF'
# component	scope	current_symbol	action	canonical_target	owner	consumers	surface_impact	evidence	decision_reference	exit_node
turn	turn	Turn	existing	Turn	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
session	session	Session	extend	Session	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
plan	plan	Plan	project	Plan	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
memory	memory	Memory	v2	Memory	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
agent	agent	Agent	new	Agent	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
command	command	Command	delete	Command	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
receipt	receipt	Receipt	existing	Receipt	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
id	id	Id	existing	Id	owner	consumer	none	crates/contracts/src/lib.rs:1	docs/decision.md#decision	X1
EOF
cat > "$x1/config/architecture/acceptance-ids.tsv" <<'EOF'
# id	node	kind	target
A-DEP-001	X1	test	crates/contracts/tests/architecture_contract.rs
A-DEP-002	X1	test	crates/contracts/tests/architecture_contract.rs
A-DEP-003	X1	test	crates/contracts/tests/architecture_contract.rs
EOF
cat > "$x1/config/architecture/fabric-public-types.tsv" <<'EOF'
# baseline_count=1
# path	kind	symbol	provenance	owner	consumers	decision_reference
crates/contracts/src/lib.rs	struct	Baseline	baseline	-	-	-
EOF
cat > "$x1/config/architecture/id-collisions.tsv" <<'EOF'
# symbol	paths	representations	decision	canonical_target	exit_node
EOF
x1_check() {
  ARCH_ROOT="$x1" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
    bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh" >/dev/null 2>&1
}
expect_x1_rejection() {
  if x1_check; then echo "expected X1 gate to reject $1" >&2; exit 1; fi
}
x1_check || {
  echo 'expected clean X1 contract fixture to pass' >&2
  ARCH_ROOT="$x1" ARCH_SKIP_PHASE0_GATES=1 ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
    bash "$ROOT/scripts/libexec/aletheon/architecture-check.sh"
  exit 1
}
printf 'pub struct Unregistered;\n' >> "$x1/crates/contracts/src/lib.rs"
expect_x1_rejection 'unregistered Fabric public type'
printf 'crates/contracts/src/lib.rs\tstruct\tUnregistered\tgoverned\t-\t-\t-\n' \
  >> "$x1/config/architecture/fabric-public-types.tsv"
expect_x1_rejection 'Fabric public type without owner consumer and decision metadata'
sed -i '$c crates/contracts/src/lib.rs\tstruct\tUnregistered\tgoverned\tfabric\tclient\tdocs/decision.md#decision' \
  "$x1/config/architecture/fabric-public-types.tsv"
x1_check || { echo 'governed Fabric public type metadata was rejected' >&2; exit 1; }
sed -i '$d' "$x1/crates/contracts/src/lib.rs"; sed -i '$d' "$x1/config/architecture/fabric-public-types.tsv"
mkdir -p "$x1/crates/alpha/src" "$x1/crates/beta/src"
printf 'pub struct DuplicateId(pub u64);\n' > "$x1/crates/alpha/src/lib.rs"
printf 'pub struct DuplicateId(pub String);\n' > "$x1/crates/beta/src/lib.rs"
expect_x1_rejection 'unregistered duplicate ID wrapper'; rm -r "$x1/crates/alpha" "$x1/crates/beta"
cat > "$x1/crates/aletheon/src/wiring/daemon/handler/rpc.rs" <<'RS'
fn dispatch(method: &str) {
    match method {
        "test.method" => (),
        "test.method" => (),
        _ => (),
    }
}
RS
expect_x1_rejection 'duplicate RPC method arm'; rm "$x1/crates/aletheon/src/wiring/daemon/handler/rpc.rs"
sed -i '/a_dep_003_fixture/d' "$x1/crates/contracts/tests/architecture_contract.rs"
expect_x1_rejection 'acceptance ID without a test function'
echo 'X1 contract negative fixtures: pass'

# Runtime authority invariants are checked against the real source tree. Keep
# the strings split where the check itself names a forbidden local path.
forbidden_root='/home/'"aurobear/Bear-ws"
local_workspace='LOCAL_'"WORKSPACE_ROOT"
legacy_workspace='LEGACY_'"WORKING_DIR"
if git -C "$ROOT" grep -nE "${local_workspace}|${legacy_workspace}|${forbidden_root}" -- crates config scripts tests; then
  echo 'forbidden fixed workspace root remains' >&2; exit 1
fi
if git -C "$ROOT" grep -n 'PrincipalId(session_id)' -- crates; then
  echo 'session id is still used as a principal' >&2; exit 1
fi
if git -C "$ROOT" grep -n 'default_session_id.lock' -- crates/executive/src/application/daemon_turn; then
  echo 'turn path still rereads the default session' >&2; exit 1
fi
if grep -qE 'ProviderRegistry|api_key|api_url' "$ROOT/crates/aletheon/src/wiring/user_runtime.rs"; then
  echo 'user runtime exposes machine provider authority' >&2; exit 1
fi
if grep -qE 'RequestHandler|ToolRegistry|Sandbox' "$ROOT/crates/aletheon/src/wiring/core_runtime.rs"; then
  echo 'system core exposes user execution authority' >&2; exit 1
fi
# The adapter registry owns the factory definition; exactly one host wiring
# file may call it, and that caller must remain the machine core.
test "$(rg -l 'resolve_and_create' "$ROOT/crates/aletheon/src/wiring" \
  -g '!**/adapters/inference/registry.rs' | wc -l)" -eq 1
rg -q 'resolve_and_create' "$ROOT/crates/aletheon/src/wiring/core_runtime.rs"
echo 'multi-user runtime architecture boundary: pass'
python3 "$ROOT/scripts/verify-approval-closure.py"

# Hotspot budgets make ownership concentration measurable. They do not claim a
# large file is defective; they prevent silent growth until responsibility is
# split or the reviewed budget/owner record is deliberately updated.
python3 - "$ROOT" "$ROOT/config/architecture/hotspot-budgets.tsv" <<'PY'
import pathlib, sys
root, manifest = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
seen = set()
for line_no, raw in enumerate(manifest.read_text().splitlines(), 1):
    if not raw or raw.startswith("#"): continue
    path, maximum, owner, responsibility = raw.split("\t", 3)
    if path in seen or not owner.strip() or not responsibility.strip():
        raise SystemExit(f"invalid hotspot ownership row {line_no}")
    seen.add(path)
    source = root / path
    if not source.is_file(): raise SystemExit(f"missing hotspot source: {path}")
    actual = sum(1 for _ in source.open("rb"))
    if actual > int(maximum):
        raise SystemExit(f"hotspot budget exceeded: {path} lines={actual} max={maximum} owner={owner}")
print(f"hotspot ownership budgets verified: {len(seen)}")
PY
