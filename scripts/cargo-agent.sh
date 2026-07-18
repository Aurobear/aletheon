#!/usr/bin/env bash
set -euo pipefail

cache_root=${ALETHEON_CARGO_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aletheon-cargo}
target_dir=${CARGO_TARGET_DIR:-$cache_root/target}
lock_file=$cache_root/build.lock
max_gib=${ALETHEON_CARGO_TARGET_MAX_GIB:-60}

[[ "$max_gib" =~ ^[1-9][0-9]*$ ]] || {
  echo "ALETHEON_CARGO_TARGET_MAX_GIB must be a positive integer" >&2
  exit 2
}

mkdir -p -- "$cache_root" "$target_dir"

# The cleanup and the following Cargo process use the same cross-worktree lock.
# This prevents both concurrent rustc storms and deleting artifacts mid-build.
exec 9>"$lock_file"
if ! flock -w "${ALETHEON_CARGO_LOCK_TIMEOUT_SEC:-1800}" 9; then
  echo "timed out waiting for Aletheon Cargo build lock: $lock_file" >&2
  exit 75
fi

size_kib=$(du -sk -- "$target_dir" 2>/dev/null | awk '{print $1+0}')
max_kib=$((max_gib * 1024 * 1024))
if (( size_kib > max_kib )); then
  echo "Aletheon Cargo target exceeds ${max_gib} GiB; cleaning $target_dir" >&2
  cargo clean --target-dir "$target_dir"
fi

export CARGO_TARGET_DIR="$target_dir"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}

exec cargo "$@"

