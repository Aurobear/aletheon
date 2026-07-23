#!/usr/bin/env bash

cmd_install() {
  local enable=1
  [[ ${1:-} == --no-enable ]] && enable=0
  [[ $# -le 1 ]] || aletheon_die "usage: aletheon.sh install [--no-enable]"
  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "missing release binary; run build first"
  local args=()
  ((enable)) || args+=(--no-enable)
  aletheon_info "installing reviewed system assets (sudo boundary)"
  sudo env ALETHEON_BINARY="$ALETHEON_RELEASE_BINARY" \
    ALETHEON_CONFIG="$ALETHEON_ROOT/config/production.toml.example" \
    bash "$ALETHEON_LIBEXEC/install-systemd.sh" "${args[@]}"
  # Migrate legacy per-user installs which shadow the reviewed unit in
  # /usr/lib/systemd/user and keep the daemon on a stale ~/.local binary.
  local user_unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  local legacy_unit="$user_unit_dir/aletheon.service"
  local legacy_binary_override="$user_unit_dir/aletheon.service.d/20-binary-path.conf"
  if [[ -f "$legacy_unit" ]] &&
     grep -qE '^ExecStart=(%h|/home/[^/]+)/\\.local/bin/aletheon daemon$' "$legacy_unit"; then
    rm -f -- "$legacy_unit"
  fi
  if [[ -f "$legacy_binary_override" ]] &&
     grep -qE '^ExecStart=(%h|/home/[^/]+)/\\.local/bin/aletheon daemon$' "$legacy_binary_override"; then
    rm -f -- "$legacy_binary_override"
  fi
  rmdir "$user_unit_dir/aletheon.service.d" 2>/dev/null || true
  systemctl --user daemon-reload
  aletheon_ok "system assets installed"
}

cmd_closure_install() {
  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  install -d -m 0755 "$bin_dir" "$unit_dir"
  install -m 0755 "$ALETHEON_LIBEXEC/pi-scheduled-task.sh" \
    "$bin_dir/aletheon-pi-scheduled-task"
  install -m 0644 "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.service" \
    "$unit_dir/aletheon-pi-closure.service"
  install -m 0644 "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.timer" \
    "$unit_dir/aletheon-pi-closure.timer"
  cmp -s "$ALETHEON_LIBEXEC/pi-scheduled-task.sh" "$bin_dir/aletheon-pi-scheduled-task"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.service" "$unit_dir/aletheon-pi-closure.service"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.timer" "$unit_dir/aletheon-pi-closure.timer"
  systemd-analyze --user verify "$unit_dir/aletheon-pi-closure.service" "$unit_dir/aletheon-pi-closure.timer"
  systemctl --user daemon-reload
  systemctl --user enable --now aletheon-pi-closure.timer
  aletheon_ok "Pi closure assets installed and timer enabled"
}
