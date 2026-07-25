#!/usr/bin/env bash

cmd_configure() {
  local action=${1:-show}
  case "$action" in
    show)
      printf 'config=%s\nuser_socket=%s\ncore_socket=%s\n' \
        "$ALETHEON_CONFIG_FILE" "$ALETHEON_USER_SOCKET" "$ALETHEON_CORE_SOCKET"
      local endpoint
      if endpoint=$(gbrain_endpoint); then printf 'gbrain_endpoint=%s\n' "$endpoint"; else printf 'gbrain_endpoint=disabled\n'; fi
      ;;
    check)
      [[ -f "$ALETHEON_CONFIG_FILE" ]] || aletheon_die "missing user config: $ALETHEON_CONFIG_FILE"
      local endpoint
      if endpoint=$(gbrain_endpoint); then validate_http_endpoint "$endpoint"; fi
      aletheon_ok "configuration paths and endpoint syntax are valid"
      ;;
    *) aletheon_die "usage: aletheon.sh configure {show|check}" ;;
  esac
}

cmd_status() {
  systemctl --no-pager --full status aletheon-core.service || true
  systemctl --user --no-pager --full status aletheon.service aletheon.socket || true
  systemctl --user --no-pager --full status aletheon-pi-closure.timer || true
  systemctl --user is-active --quiet aletheon.service
  systemctl is-active --quiet aletheon-core.service
  systemctl --user is-active --quiet aletheon-pi-closure.timer
}

cmd_restart() {
  aletheon_info "restarting machine core and user daemon"
  # The installer may have just restarted the core while replacing units.
  # Clear systemd's rate-limit accounting before the deliberate deploy restart.
  sudo systemctl reset-failed aletheon-core.service
  systemctl --user reset-failed aletheon.service 2>/dev/null || true
  sudo systemctl restart aletheon-core.service
  systemctl --user restart aletheon.service
  aletheon_ok "services restarted"
}

cmd_restart_user() {
  aletheon_info "restarting user daemon (rootless, no core)"
  systemctl --user reset-failed aletheon.service 2>/dev/null || true
  systemctl --user restart aletheon.service
  aletheon_ok "user daemon restarted"
}

cmd_logs() {
  case "${1:-user}" in
    core) journalctl -u aletheon-core.service -n "${ALETHEON_LOG_LINES:-100}" --no-pager ;;
    user) journalctl --user -u aletheon.service -n "${ALETHEON_LOG_LINES:-100}" --no-pager ;;
    closure) journalctl --user -u aletheon-pi-closure.service -n "${ALETHEON_LOG_LINES:-100}" --no-pager ;;
    *) aletheon_die "usage: aletheon.sh logs [core|user|closure]" ;;
  esac
}

cmd_closure() {
  case "${1:-status}" in
    install) cmd_closure_install ;;
    run) systemctl --user start aletheon-pi-closure.service ;;
    status) systemctl --user --no-pager --full status aletheon-pi-closure.timer aletheon-pi-closure.service || true ;;
    *) aletheon_die "usage: aletheon.sh closure {install|run|status}" ;;
  esac
}
