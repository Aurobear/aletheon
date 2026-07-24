#!/usr/bin/env bash

_verify_gbrain_health() {
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

cmd_health() {
  "$ALETHEON_LIBEXEC/healthcheck.sh" --core-socket "$ALETHEON_CORE_SOCKET"
  "$ALETHEON_LIBEXEC/healthcheck.sh" --user-socket "$ALETHEON_USER_SOCKET"
  _verify_gbrain_health
}

cmd_verify() {
  cmd_configure check
  systemctl is-active --quiet aletheon-core.service
  systemctl --user is-active --quiet aletheon.service
  systemctl --user is-active --quiet aletheon-pi-closure.timer
  cmd_health
  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  cmp -s "$ALETHEON_LIBEXEC/pi-scheduled-task.sh" "$bin_dir/aletheon-pi-scheduled-task"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.service" "$unit_dir/aletheon-pi-closure.service"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.timer" "$unit_dir/aletheon-pi-closure.timer"
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi coding runtime registered' -n 1 --quiet
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi resident RPC runtime registered' -n 1 --quiet
  cmd_installed_runtime_gate
  aletheon_ok "deployment verification passed"
}

cmd_verify_user() {
  local bin_dir=${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}
  local unit_dir=${ALETHEON_USER_UNIT_DIR:-$HOME/.config/systemd/user}
  cmd_configure check
  systemctl --user is-active --quiet aletheon.service
  systemctl --user is-active --quiet aletheon-pi-closure.timer
  "$ALETHEON_LIBEXEC/healthcheck.sh" --user-socket "$ALETHEON_USER_SOCKET"
  _verify_gbrain_health
  cmp -s "$ALETHEON_LIBEXEC/pi-scheduled-task.sh" "$bin_dir/aletheon-pi-scheduled-task"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.service" "$unit_dir/aletheon-pi-closure.service"
  cmp -s "$ALETHEON_ROOT/deploy/systemd/user/aletheon-pi-closure.timer" "$unit_dir/aletheon-pi-closure.timer"
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi coding runtime registered' -n 1 --quiet
  journalctl --user -u aletheon.service -b --no-pager --grep='Pi resident RPC runtime registered' -n 1 --quiet
  cmd_installed_runtime_gate_user "$bin_dir/aletheon"
  aletheon_ok "user-mode deployment verification passed"
}

cmd_verify_specialized() {
  local target=${1:-}
  [[ -n "$target" ]] && shift
  case "$target" in
    systemd) run_internal verify/systemd.sh "$@" ;;
    network) run_internal verify/network-exposure.sh "$@" ;;
    compose) run_internal verify/compose.sh "$@" ;;
    migration) run_internal verify/migration-matrix.sh "$@" ;;
    multi-user) run_internal verify/multi-user-runtime.sh "$@" ;;
    *) aletheon_die "usage: aletheon.sh verify {systemd|network|compose|migration|multi-user} [options]" || return 2 ;;
  esac
}
