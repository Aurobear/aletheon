#!/usr/bin/env bash

aletheon_build_revision() {
  local head dirty_hash untracked
  head=$(git -C "$ALETHEON_ROOT" rev-parse HEAD) || return
  untracked=$(git -C "$ALETHEON_ROOT" ls-files --others --exclude-standard -- \
    Cargo.toml Cargo.lock crates config scripts)
  if git -C "$ALETHEON_ROOT" diff --quiet HEAD -- \
      Cargo.toml Cargo.lock crates config scripts && [[ -z "$untracked" ]]; then
    printf '%s\n' "$head"
    return
  fi
  dirty_hash=$(
    {
      git -C "$ALETHEON_ROOT" diff --binary HEAD -- \
        Cargo.toml Cargo.lock crates config scripts
      while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        printf 'untracked %s ' "$path"
        sha256sum "$ALETHEON_ROOT/$path"
      done <<<"$untracked"
    } | sha256sum | awk '{print substr($1, 1, 16)}'
  )
  printf '%s-dirty-%s\n' "$head" "$dirty_hash"
}

aletheon_build_config_hash() {
  sha256sum \
    "$ALETHEON_ROOT/config/default.toml" \
    "$ALETHEON_ROOT/config/production.toml.example" |
    sha256sum | awk '{print $1}'
}

cmd_build() {
  aletheon_info "building release binary in the shared incremental Cargo target"
  local build_revision config_hash
  build_revision=${GIT_COMMIT_SHA:-$(aletheon_build_revision)}
  config_hash=${CONFIG_HASH:-$(aletheon_build_config_hash)}
  [[ -n "$build_revision" && -n "$config_hash" ]] ||
    aletheon_die "could not derive build provenance"
  aletheon_info "build provenance: $build_revision"

  # The public entrypoint normally re-enters as SUDO_USER before reaching this
  # function. Keep this fallback for callers that source the module directly.
  if [[ -n "${SUDO_USER:-}" ]] && [[ "$(whoami)" == "root" ]]; then
    aletheon_info "running as root (sudo) — building as $SUDO_USER to reuse cargo cache"
    sudo -u "$SUDO_USER" -H env \
      ALETHEON_CARGO_CACHE_ROOT="$ALETHEON_CARGO_CACHE_ROOT" \
      CARGO_TARGET_DIR="$ALETHEON_BUILD_TARGET_DIR" \
      ALETHEON_CARGO_STAGE_SOURCE="$ALETHEON_BUILD_BINARY" \
      ALETHEON_CARGO_STAGE_BINARY="$ALETHEON_RELEASE_BINARY" \
      GIT_COMMIT_SHA="$build_revision" \
      CONFIG_HASH="$config_hash" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  else
    CARGO_TARGET_DIR="$ALETHEON_BUILD_TARGET_DIR" \
      ALETHEON_CARGO_STAGE_SOURCE="$ALETHEON_BUILD_BINARY" \
      ALETHEON_CARGO_STAGE_BINARY="$ALETHEON_RELEASE_BINARY" \
      GIT_COMMIT_SHA="$build_revision" \
      CONFIG_HASH="$config_hash" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  fi

  [[ -x "$ALETHEON_BUILD_BINARY" ]] ||
    aletheon_die "release binary was not produced: $ALETHEON_BUILD_BINARY"

  # cargo-agent stages the candidate before releasing its cross-worktree lock,
  # so another checkout cannot replace the shared output between build and copy.
  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "release candidate staging failed"
  aletheon_ok "release binary ready: $ALETHEON_RELEASE_BINARY (cache: $ALETHEON_BUILD_TARGET_DIR)"
}
