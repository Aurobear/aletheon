#!/usr/bin/env bash

cmd_monitor_install() {
  local source_dir="$ALETHEON_ROOT/tools/aletheon-monitor"
  [[ -d "$source_dir" ]] || {
    aletheon_warn "monitor source is unavailable; skipping MCP monitor install"
    return 0
  }

  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local data_dir=${ALETHEON_USER_DATA_DIR:-$HOME/.local/share/aletheon}
  local monitor_dir="$data_dir/monitor"
  local venv_dir="$monitor_dir/.venv"
  local dependency_stamp="$monitor_dir/.dependency-spec.sha256"
  local expected_stamp
  expected_stamp=$(sha256sum "$source_dir/pyproject.toml" | awk '{print $1}')

  aletheon_info "installing Aletheon monitor in an isolated Python environment"
  install -d -m 0755 "$bin_dir" "$monitor_dir"
  cp -a "$source_dir"/. "$monitor_dir/"

  if [[ ! -x "$venv_dir/bin/python" ]]; then
    rm -rf -- "$venv_dir"
    python3 -m venv "$venv_dir"
  fi
  if [[ ! -f "$dependency_stamp" ]] ||
     [[ $(cat "$dependency_stamp") != "$expected_stamp" ]] ||
     ! "$venv_dir/bin/python" -c 'import mcp' >/dev/null 2>&1; then
    "$venv_dir/bin/python" -m pip install --disable-pip-version-check "$monitor_dir"
    printf '%s\n' "$expected_stamp" > "$dependency_stamp"
  fi

  cat > "$bin_dir/aletheon-monitor" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if [[ -f /etc/aletheon/.env ]]; then
  set -a; source /etc/aletheon/.env; set +a
elif [[ -f "\$HOME/.config/aletheon/.env" ]]; then
  set -a; source "\$HOME/.config/aletheon/.env"; set +a
fi

exec "$venv_dir/bin/python" "$monitor_dir/run.py" "\$@"
EOF
  chmod 0755 "$bin_dir/aletheon-monitor"
  "$venv_dir/bin/python" -c \
    "import sys; sys.path.insert(0, '$monitor_dir'); from src.server import server"
  aletheon_ok "MCP monitor installed at $bin_dir/aletheon-monitor"
}

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
     grep -qE '^ExecStart=(%h|/home/[^/]+)/\.local/bin/aletheon daemon$' "$legacy_unit"; then
    rm -f -- "$legacy_unit"
  fi
  if [[ -f "$legacy_binary_override" ]] &&
     grep -qE '^ExecStart=(%h|/home/[^/]+)/\.local/bin/aletheon daemon$' "$legacy_binary_override"; then
    rm -f -- "$legacy_binary_override"
  fi
  rmdir "$user_unit_dir/aletheon.service.d" 2>/dev/null || true
  systemctl --user daemon-reload
  cmd_monitor_install
  aletheon_ok "system assets installed"
}

cmd_install_user() {
  local enable=1
  [[ ${1:-} == --no-enable ]] && enable=0
  [[ $# -le 1 ]] || aletheon_die "usage: aletheon.sh install-user [--no-enable]"
  [[ -x "$ALETHEON_RELEASE_BINARY" ]] || aletheon_die "missing release binary; run build first"
  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  aletheon_info "installing user-mode assets under \$HOME (rootless)"
  install -d -m 0755 "$bin_dir" "$unit_dir"
  install -m 0755 "$ALETHEON_RELEASE_BINARY" "$bin_dir/aletheon"
  # The reviewed unit ships with a %h-relative ExecStart; pin it to the concrete
  # install directory so the running executable path matches provenance checks.
  sed "s|ExecStart=%h/.local/bin/aletheon daemon|ExecStart=$bin_dir/aletheon daemon|" \
    "$ALETHEON_ROOT/config/aletheon.user.service" > "$unit_dir/aletheon.service"
  install -m 0644 "$ALETHEON_ROOT/config/aletheon.user.socket" "$unit_dir/aletheon.socket"
  systemd-analyze --user verify "$unit_dir/aletheon.service" "$unit_dir/aletheon.socket"
  systemctl --user daemon-reload
  if ((enable)); then
    systemctl --user enable --now aletheon.socket
  fi
  [[ -f "$ALETHEON_CONFIG_FILE" ]] ||
    aletheon_warn "no user config at $ALETHEON_CONFIG_FILE; run ./setup.sh --user or create it before first turn"
  cmd_monitor_install
  aletheon_ok "user-mode assets installed under $bin_dir and $unit_dir"
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
