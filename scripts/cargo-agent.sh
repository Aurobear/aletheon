#!/usr/bin/env bash
set -euo pipefail

# Ensure cargo is on PATH (sudo strips it).
export PATH="$HOME/.cargo/bin:$PATH"

cache_root=${ALETHEON_CARGO_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aletheon-cargo}
target_dir=${CARGO_TARGET_DIR:-$cache_root/target}
lock_file=$cache_root/build.lock
max_gib=${ALETHEON_CARGO_TARGET_MAX_GIB:-60}

[[ "$max_gib" =~ ^[1-9][0-9]*$ ]] || {
  echo "ALETHEON_CARGO_TARGET_MAX_GIB must be a positive integer" >&2
  exit 2
}

export CARGO_TARGET_DIR="$target_dir"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}
export ALETHEON_CARGO_TARGET_MAX_GIB="$max_gib"

mkdir -p -- "$cache_root" "$target_dir"

# `cargo metadata` only reads manifests and the lockfile. It neither compiles
# nor mutates the shared target directory, so serializing it behind a long
# workspace test makes independent architecture jobs appear stalled without
# protecting any build artifact.
if [[ ${1:-} == metadata || ( ${1:-} == +* && ${2:-} == metadata ) ]]; then
  exec cargo "$@"
fi

# Keep the lock in the supervising `flock` process and close its descriptor in
# Cargo. Otherwise hook/test subprocesses can inherit the descriptor, outlive
# Cargo, and make every later agent wait forever on an apparently stale lock.
# Cleanup and Cargo still run under the same cross-worktree exclusive lock.
exec flock -w "${ALETHEON_CARGO_LOCK_TIMEOUT_SEC:-1800}" -o "$lock_file" \
  bash -c '
    set -euo pipefail
    size_kib=$(du -sk -- "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)
    max_kib=$((ALETHEON_CARGO_TARGET_MAX_GIB * 1024 * 1024))
    if (( size_kib > max_kib )); then
      echo "Aletheon Cargo target exceeds ${ALETHEON_CARGO_TARGET_MAX_GIB} GiB; cleaning $CARGO_TARGET_DIR" >&2
      # Cargo refuses to clean a target directory whose cache tag is missing,
      # even when Cargo itself populated that directory. Restore the standard
      # tag so bounded-cache cleanup cannot turn a successful build into a
      # permanent failure loop.
      printf "%s\n" \
        "Signature: 8a477f597d28d172789f06886806bc55" \
        "# This file is a cache directory tag created by cargo." \
        "# For information about cache directory tags see https://bford.info/cachedir/" \
        > "$CARGO_TARGET_DIR/CACHEDIR.TAG"
      cargo clean --target-dir "$CARGO_TARGET_DIR"
    fi
    exec cargo "$@"
  ' cargo-agent "$@"
