#!/usr/bin/env bash
set -euo pipefail

cache_root=${ALETHEON_CARGO_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aletheon-cargo}
target_dir=${CARGO_TARGET_DIR:-$cache_root/target}
lock_file=$cache_root/build.lock

mkdir -p -- "$cache_root"
exec 9>"$lock_file"
if ! flock -w "${ALETHEON_CARGO_LOCK_TIMEOUT_SEC:-1800}" 9; then
  echo "timed out waiting for Aletheon Cargo build lock: $lock_file" >&2
  exit 75
fi

if [[ ! -d "$target_dir" ]]; then
  echo "Cargo target does not exist: $target_dir" >&2
  exit 0
fi

echo "Cleaning managed Cargo target: $target_dir" >&2
cargo clean --target-dir "$target_dir"

