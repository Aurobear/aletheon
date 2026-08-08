#!/usr/bin/env bash
set -euo pipefail

# Ensure cargo is on PATH (sudo strips it).
export PATH="$HOME/.cargo/bin:$PATH"

cache_root=${ALETHEON_CARGO_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aletheon-cargo}
target_dir=${CARGO_TARGET_DIR:-$cache_root/target}
lock_file=$cache_root/build.lock
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
max_gib=${ALETHEON_CARGO_TARGET_MAX_GIB:-60}
scan_interval_sec=${ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC:-900}
force_size_scan=${ALETHEON_CARGO_TARGET_FORCE_SCAN:-0}

[[ "$max_gib" =~ ^[1-9][0-9]*$ ]] || {
  echo "ALETHEON_CARGO_TARGET_MAX_GIB must be a positive integer" >&2
  exit 2
}
[[ "$scan_interval_sec" =~ ^[0-9]+$ ]] || {
  echo "ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC must be a non-negative integer" >&2
  exit 2
}
[[ "$force_size_scan" == 0 || "$force_size_scan" == 1 ]] || {
  echo "ALETHEON_CARGO_TARGET_FORCE_SCAN must be 0 or 1" >&2
  exit 2
}

export CARGO_TARGET_DIR="$target_dir"

# Size compilation concurrency from both installed and currently available
# memory instead of permanently under-utilising developer machines. The total
# memory guard reserves roughly 4 GiB per worker for peak rustc processes; the
# availability guard permits one worker per 3 GiB while memory is otherwise
# idle. On the 32 GB development class this selects six workers, but backs off
# automatically under pressure. Callers can still provide CARGO_BUILD_JOBS.
if [[ -z ${CARGO_BUILD_JOBS:-} ]]; then
  cpu_jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '2\n')
  total_kib=$(awk '/^MemTotal:/ { print $2; exit }' /proc/meminfo 2>/dev/null || true)
  available_kib=$(awk '/^MemAvailable:/ { print $2; exit }' /proc/meminfo 2>/dev/null || true)
  if [[ "$cpu_jobs" =~ ^[1-9][0-9]*$ \
    && "$total_kib" =~ ^[1-9][0-9]*$ \
    && "$available_kib" =~ ^[1-9][0-9]*$ ]]; then
    total_memory_jobs=$((total_kib / (4 * 1024 * 1024)))
    available_memory_jobs=$((available_kib / (3 * 1024 * 1024)))
    (( total_memory_jobs >= 1 )) || total_memory_jobs=1
    (( available_memory_jobs >= 1 )) || available_memory_jobs=1
    (( cpu_jobs <= total_memory_jobs )) || cpu_jobs=$total_memory_jobs
    (( cpu_jobs <= available_memory_jobs )) || cpu_jobs=$available_memory_jobs
    (( cpu_jobs <= 8 )) || cpu_jobs=8
    export CARGO_BUILD_JOBS=$cpu_jobs
  else
    export CARGO_BUILD_JOBS=2
  fi
fi
export ALETHEON_CARGO_TARGET_MAX_GIB="$max_gib"
export ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC="$scan_interval_sec"
export ALETHEON_CARGO_TARGET_FORCE_SCAN="$force_size_scan"

mkdir -p -- "$cache_root" "$target_dir"
target_key=$(printf '%s' "$target_dir" | sha256sum | cut -c1-16)
export ALETHEON_CARGO_TARGET_SCAN_STAMP="$cache_root/target-size-$target_key.stamp"

# A bounded compiler cache preserves reusable objects even when Cargo must
# rotate its target directory. Explicit RUSTC_WRAPPER/SCCACHE settings win.
if [[ -z ${RUSTC_WRAPPER+x} ]] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER="$script_dir/libexec/aletheon/rustc-sccache-fallback.sh"
  export SCCACHE_DIR=${SCCACHE_DIR:-$cache_root/sccache}
  export SCCACHE_CACHE_SIZE=${SCCACHE_CACHE_SIZE:-20G}
  # A self-hosted runner can reap or restart its inherited sccache daemon while
  # another serialized build is starting. Preserve compilation correctness by
  # retrying the compiler locally after a proven server transport failure;
  # callers that explicitly configure the policy still win. The wrapper is
  # required because some deployed sccache versions do not honor the native
  # fallback setting for failures that occur after connecting to the server.
  export SCCACHE_IGNORE_SERVER_IO_ERROR=${SCCACHE_IGNORE_SERVER_IO_ERROR:-1}
  mkdir -p -- "$SCCACHE_DIR"
fi

# GNU ld spent most of an all-feature test build repeatedly linking hundreds
# of large integration binaries. Prefer mold when installed, while preserving
# caller-supplied Rust flags and allowing an explicit opt-out.
linker_label=system
if [[ ${ALETHEON_CARGO_FAST_LINKER:-auto} != off ]] \
  && [[ -z ${CARGO_ENCODED_RUSTFLAGS:-} ]] \
  && [[ -z ${RUSTFLAGS:-} ]] \
  && command -v mold >/dev/null 2>&1; then
  export RUSTFLAGS="-C link-arg=-fuse-ld=mold"
  linker_label=mold
fi

printf 'Aletheon Cargo: jobs=%s incremental=%s linker=%s rustc_wrapper=%s target=%s\n' \
  "$CARGO_BUILD_JOBS" "${CARGO_INCREMENTAL:-profile-default}" "$linker_label" \
  "${RUSTC_WRAPPER:-none}" "$CARGO_TARGET_DIR" >&2

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
# A recursive `du` over a large incremental target can itself take tens of
# seconds, so perform it periodically rather than charging every no-op check.
# Set ALETHEON_CARGO_TARGET_FORCE_SCAN=1 (or the interval to 0) for an immediate
# capacity audit. The previous behavior also checked only before a command, so
# a single oversized build was always cleaned on a subsequent invocation.
if ! flock -n "$lock_file" true; then
  echo "Aletheon Cargo: another build owns the shared compilation lock; waiting" >&2
fi
exec flock -w "${ALETHEON_CARGO_LOCK_TIMEOUT_SEC:-1800}" -o "$lock_file" \
  bash -c '
    set -euo pipefail
    scan_due=0
    if [[ "$ALETHEON_CARGO_TARGET_FORCE_SCAN" == 1 \
      || "$ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC" == 0 \
      || ! -e "$ALETHEON_CARGO_TARGET_SCAN_STAMP" ]]; then
      scan_due=1
    else
      now=$(date +%s)
      last_scan=$(stat -c %Y -- "$ALETHEON_CARGO_TARGET_SCAN_STAMP" 2>/dev/null || printf "0\n")
      if (( now < last_scan || now - last_scan >= ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC )); then
        scan_due=1
      fi
    fi

    if (( scan_due )); then
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
      touch -- "$ALETHEON_CARGO_TARGET_SCAN_STAMP"
    fi
    exec cargo "$@"
  ' cargo-agent "$@"
