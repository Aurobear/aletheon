#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/config" "$tmp/target" \
  "$tmp/crates/corpus/src/legacy" "$tmp/crates/dasein/src" \
  "$tmp/crates/executive/src" "$tmp/crates/interact/src" "$tmp/crates/fabric/src"
printf 'pub use example::Example;\n' > "$tmp/crates/fabric/src/lib.rs"
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
use fabric::envelope::Envelope;
use executive::impl::kernel::Table;
fn fields(x: X) { let _ = x.runtime; }
RS
cat > "$tmp/config/architecture-allowlist.txt" <<'BASE'
concrete_clock|crates/dasein/src/lib.rs|fn clock() { SystemClock::new(); }
core_systems_field|crates/executive/src/lib.rs|fn fields(x: X) { let _ = x.runtime; }
direct_tool|crates/corpus/src/legacy/mod.rs|tool.execute(x)
duplicate_kernel|crates/executive/src/lib.rs|use executive::impl::kernel::Table;
legacy_event|crates/executive/src/lib.rs|use fabric::envelope::Envelope;
BASE
: > "$tmp/config/architecture-dependencies.txt"
: > "$tmp/config/architecture-path-inventory.txt"
ARCH_ROOT="$tmp" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/architecture-check.sh" >/dev/null
printf 'tool.execute(y)\n' >> "$tmp/crates/corpus/src/legacy/mod.rs"
if ARCH_ROOT="$tmp" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a new finding to fail' >&2; exit 1
fi
sed -i '$d' "$tmp/crates/corpus/src/legacy/mod.rs"
rm "$tmp/crates/dasein/src/lib.rs"
out=$(ARCH_ROOT="$tmp" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
  bash "$ROOT/scripts/architecture-check.sh")
grep -q 'resolved findings entries' <<<"$out"

# A local dependency edge not present in the maximum baseline must also fail.
deps="$tmp/deps"; mkdir -p "$deps/config" "$deps/target" "$deps/crates/fabric/src" "$deps/crates/kernel/src"
cat > "$deps/Cargo.toml" <<'TOML'
[workspace]
resolver = "2"
members = ["crates/fabric", "crates/kernel"]
TOML
printf 'pub fn fabric() {}\n' > "$deps/crates/fabric/src/lib.rs"
printf 'pub fn kernel() {}\n' > "$deps/crates/kernel/src/lib.rs"
cat > "$deps/crates/fabric/Cargo.toml" <<'TOML'
[package]
name = "fabric"
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
if ARCH_ROOT="$deps" ARCH_SKIP_DELETION_GATES=1 \
  bash "$ROOT/scripts/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a new dependency to fail' >&2; exit 1
fi

# Workspace package names are semantic domain names. Do not permit the deleted
# api/types/broker/platform-* split-crate convention (or any other hyphenated
# package name) to return.
sed -i '/^\[dependencies\]/,$d' "$deps/crates/fabric/Cargo.toml"
sed -i 's/name = "kernel"/name = "runtime-api"/' "$deps/crates/kernel/Cargo.toml"
if ARCH_ROOT="$deps" ARCH_SKIP_DELETION_GATES=1 \
  bash "$ROOT/scripts/architecture-check.sh" >/dev/null 2>&1; then
  echo 'expected a hyphenated workspace package to fail' >&2; exit 1
fi

# Phase 0 semantic gates are fixture-driven.  The clean fixture deliberately
# contains the two legal exceptions: adapter-id selection in composition and
# opaque JSON inspection inside an adapter.
phase0="$tmp/phase0"
mkdir -p "$phase0/config/architecture" "$phase0/config" "$phase0/target" \
  "$phase0/crates/fabric/src/protocol" "$phase0/crates/fabric/src/application" \
  "$phase0/crates/fabric/src/composition" "$phase0/crates/fabric/src/adapter"
cat > "$phase0/Cargo.toml" <<'TOML'
[workspace]
resolver = "2"
members = ["crates/fabric"]
TOML
cat > "$phase0/crates/fabric/Cargo.toml" <<'TOML'
[package]
name = "fabric"
version = "0.1.0"
edition = "2021"
TOML
cat > "$phase0/crates/fabric/src/lib.rs" <<'RS'
pub fn stable_contract() {}
RS
printf 'pub struct Request;\n' > "$phase0/crates/fabric/src/protocol/client.rs"
cat > "$phase0/crates/fabric/src/composition/registry.rs" <<'RS'
fn construct(adapter_id: &str) { match adapter_id { "messages-http" => (), _ => () } }
RS
cat > "$phase0/crates/fabric/src/adapter/json.rs" <<'RS'
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
fabric|crates/fabric|-|protocol|false|adapter
EOF
cat > "$phase0/config/architecture/external-identifiers.txt" <<'EOF'
# frozen_commit=fixture
evil	\bEvilCorp\b	crates/fabric/src/adapter/	fixture external name	neutral contract	1
EOF
cat > "$phase0/config/architecture/wire-surfaces.tsv" <<'EOF'
# frozen_commit=fixture
wire-exposed	Request	crates/fabric/src/protocol/client.rs	client,server	fabric	v1	additive	1
EOF
cat > "$phase0/config/architecture/persistence-surfaces.tsv" <<'EOF'
# frozen_commit=fixture
fixture	fabric	crates/fabric/src/store.rs	v1	reader	writer	versioned	1
EOF
cat > "$phase0/config/architecture/compatibility-debt.tsv" <<'EOF'
# frozen_commit=fixture
legacy	crates/fabric/src/lib.rs	LEGACY	fixture debt	stable contract	0	1
EOF
cat > "$phase0/config/architecture/metrics.env" <<'EOF'
# frozen_commit=fixture
CORE_EXTERNAL_IDENTIFIER_HITS=0
CORE_OPAQUE_VALUE_INSPECTIONS=0
CROSS_CRATE_IMPL_REFERENCES=0
FORBIDDEN_INFRA_IMPORTS=0
PROVIDER_ERROR_TEXT_BRANCHES=0
PROVIDER_NAME_BRANCHES=0
PUBLIC_IMPL_ADAPTER_EXPORTS=0
URL_PROVIDER_INFERENCE=0
EOF
phase0_check() {
  ARCH_ROOT="$phase0" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
    bash "$ROOT/scripts/architecture-check.sh" >/dev/null 2>&1
}
phase0_check || {
  echo 'expected clean Phase 0 fixture to pass' >&2
  ARCH_ROOT="$phase0" ARCH_SKIP_DELETION_GATES=1 ARCH_SKIP_DEPENDENCIES=1 \
    bash "$ROOT/scripts/architecture-check.sh"
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
sed -i 's#members = \["crates/fabric"\]#members = ["crates/fabric", "crates/extra"]#' "$phase0/Cargo.toml"
expect_phase0_rejection 'unregistered workspace crate'
sed -i 's#members = \["crates/fabric", "crates/extra"\]#members = ["crates/fabric"]#' "$phase0/Cargo.toml"; rm -r "$phase0/crates/extra"
mkdir -p "$phase0/crates/fabric/src/impl"
expect_phase0_rejection 'unregistered top-level impl tree'; rmdir "$phase0/crates/fabric/src/impl"
printf 'fn leak() { let _ = EvilCorp::new(); }\n' > "$phase0/crates/fabric/src/application/leak.rs"
expect_phase0_rejection 'external name in core'; rm "$phase0/crates/fabric/src/application/leak.rs"
printf 'use executive::adapter::Store;\n' > "$phase0/crates/fabric/src/application/leak.rs"
expect_phase0_rejection 'application adapter import'; rm "$phase0/crates/fabric/src/application/leak.rs"
printf 'pub mod adapter;\n' >> "$phase0/crates/fabric/src/lib.rs"
expect_phase0_rejection 'public adapter export'; sed -i '$d' "$phase0/crates/fabric/src/lib.rs"
printf 'fn choose(provider: &str) { if provider == "evil" {} }\n' > "$phase0/crates/fabric/src/application/leak.rs"
expect_phase0_rejection 'provider-name business branch'; rm "$phase0/crates/fabric/src/application/leak.rs"
printf 'pub struct NewWire;\n' > "$phase0/crates/fabric/src/protocol/new_wire.rs"
expect_phase0_rejection 'unregistered wire surface'; rm "$phase0/crates/fabric/src/protocol/new_wire.rs"
mkdir -p "$phase0/crates/fabric/src/migrations"; printf 'SELECT 1;\n' > "$phase0/crates/fabric/src/migrations/001.sql"
expect_phase0_rejection 'unregistered persistence migration'; rm -r "$phase0/crates/fabric/src/migrations"
printf '// LEGACY\n' >> "$phase0/crates/fabric/src/lib.rs"
expect_phase0_rejection 'compatibility debt growth'; sed -i '$d' "$phase0/crates/fabric/src/lib.rs"
printf 'fn inspect(value: serde_json::Value) { let _ = value.get("business_kind"); }\n' > "$phase0/crates/fabric/src/application/leak.rs"
expect_phase0_rejection 'opaque JSON field inspection in core'; rm "$phase0/crates/fabric/src/application/leak.rs"
phase0_check || { echo 'legal composition/adapter exceptions regressed' >&2; exit 1; }
echo 'Phase 0 architecture fixtures: pass'
echo 'architecture-check fixture: pass'

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
if git -C "$ROOT" grep -n 'default_session_id.lock' -- crates/executive/src/service/daemon_turn; then
  echo 'turn path still rereads the default session' >&2; exit 1
fi
if grep -qE 'ProviderRegistry|api_key|api_url' "$ROOT/crates/executive/src/user_runtime/mod.rs"; then
  echo 'user runtime exposes machine provider authority' >&2; exit 1
fi
if grep -qE 'RequestHandler|ToolRegistry|Sandbox' "$ROOT/crates/executive/src/core/system_core_runtime.rs"; then
  echo 'system core exposes user execution authority' >&2; exit 1
fi
test "$(git -C "$ROOT" grep -l 'resolve_and_create' -- crates/executive/src | wc -l)" -eq 1
git -C "$ROOT" grep -q 'resolve_and_create' -- crates/executive/src/core/system_core_runtime.rs
echo 'multi-user runtime architecture boundary: pass'
