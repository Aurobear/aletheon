#!/usr/bin/env bash

cmd_secrets() {
  local action=${1:-}
  [[ -n "$action" ]] && shift
  case "$action" in
    init) run_internal secret-init.sh "$@" ;;
    audit) run_internal secret-audit.sh "$@" ;;
    *) aletheon_die "usage: aletheon.sh secrets {init|audit} [options]" || return 2 ;;
  esac
}

cmd_database() {
  local action=${1:-}
  [[ -n "$action" ]] && shift
  case "$action" in
    check) run_internal sqlite-check.sh "$@" ;;
    *) aletheon_die "usage: aletheon.sh database check DATABASE [DATABASE ...]" || return 2 ;;
  esac
}
