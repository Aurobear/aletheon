#!/usr/bin/env bash

cmd_build() {
  aletheon_info "building release binary through the bounded Cargo wrapper"
  CARGO_TARGET_DIR="$ALETHEON_ROOT/target" \
    bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" build -p aletheon --release
  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "release binary was not produced"
  aletheon_ok "release binary ready: $ALETHEON_RELEASE_BINARY"
}
