#!/usr/bin/env bash

cmd_build() {
  aletheon_info "building release binary through the bounded Cargo wrapper"

  # When running via sudo, cargo must run as the original user.
  # Otherwise $HOME=/root and cargo has no registry cache → full rebuild every time.
  if [[ -n "${SUDO_USER:-}" ]] && [[ "$(whoami)" == "root" ]]; then
    aletheon_info "running as root (sudo) — building as $SUDO_USER to reuse cargo cache"
    chown -R "$SUDO_USER:$SUDO_USER" "$ALETHEON_ROOT/target/" 2>/dev/null || true
    sudo -u "$SUDO_USER" env CARGO_TARGET_DIR="$ALETHEON_ROOT/target" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  else
    CARGO_TARGET_DIR="$ALETHEON_ROOT/target" \
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  fi

  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "release binary was not produced"
  aletheon_ok "release binary ready: $ALETHEON_RELEASE_BINARY"
}
