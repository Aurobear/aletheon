#!/usr/bin/env bash

cmd_build() {
  aletheon_info "building release binary in the shared incremental Cargo target"

  # The public entrypoint normally re-enters as SUDO_USER before reaching this
  # function. Keep this fallback for callers that source the module directly.
  if [[ -n "${SUDO_USER:-}" ]] && [[ "$(whoami)" == "root" ]]; then
    aletheon_info "running as root (sudo) — building as $SUDO_USER to reuse cargo cache"
    sudo -u "$SUDO_USER" -H env \
      ALETHEON_CARGO_CACHE_ROOT="$ALETHEON_CARGO_CACHE_ROOT" \
      CARGO_TARGET_DIR="$ALETHEON_BUILD_TARGET_DIR" \
      ALETHEON_CARGO_STAGE_SOURCE="$ALETHEON_BUILD_BINARY" \
      ALETHEON_CARGO_STAGE_BINARY="$ALETHEON_RELEASE_BINARY" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  else
    CARGO_TARGET_DIR="$ALETHEON_BUILD_TARGET_DIR" \
      ALETHEON_CARGO_STAGE_SOURCE="$ALETHEON_BUILD_BINARY" \
      ALETHEON_CARGO_STAGE_BINARY="$ALETHEON_RELEASE_BINARY" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  fi

  [[ -x "$ALETHEON_BUILD_BINARY" ]] ||
    aletheon_die "release binary was not produced: $ALETHEON_BUILD_BINARY"

  # cargo-agent stages the candidate before releasing its cross-worktree lock,
  # so another checkout cannot replace the shared output between build and copy.
  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "release candidate staging failed"
  aletheon_ok "release binary ready: $ALETHEON_RELEASE_BINARY (cache: $ALETHEON_BUILD_TARGET_DIR)"
}
