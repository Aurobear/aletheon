#!/usr/bin/env bash

cmd_health() {
  "$ALETHEON_ROOT/scripts/aletheon-healthcheck.sh" --core-socket "$ALETHEON_CORE_SOCKET"
  "$ALETHEON_ROOT/scripts/aletheon-healthcheck.sh" --user-socket "$ALETHEON_USER_SOCKET"
  local endpoint health
  if endpoint=$(gbrain_endpoint); then
    validate_http_endpoint "$endpoint"
    health=$(gbrain_health_url "$endpoint")
    if ! curl --fail --silent --show-error --max-time 5 "$health"; then
      aletheon_warn "GBrain is unavailable at the configured endpoint"
      return 1
    fi
    printf '\n'
  else
    aletheon_warn "GBrain MCP is not configured"
  fi
}

cmd_verify() {
  cmd_configure check
  systemctl is-active --quiet aletheon-core.service
  systemctl --user is-active --quiet aletheon.service
  systemctl --user is-active --quiet aletheon-pi-closure.timer
  cmd_health
  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  cmp -s "$ALETHEON_ROOT/scripts/aletheon-pi-scheduled-task.sh" "$bin_dir/aletheon-pi-scheduled-task"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.service" "$unit_dir/aletheon-pi-closure.service"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.timer" "$unit_dir/aletheon-pi-closure.timer"
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi coding runtime registered' -n 1 --quiet
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi resident RPC runtime registered' -n 1 --quiet
  aletheon_ok "deployment verification passed"
}
