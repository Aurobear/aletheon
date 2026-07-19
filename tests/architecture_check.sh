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
