#!/usr/bin/env bash

cmd_backup() { run_internal backup.sh "$@"; }
cmd_restore() { run_internal restore.sh "$@"; }
cmd_upgrade() { run_internal upgrade.sh "$@"; }

cmd_cleanup() {
  local target=${1:-}
  [[ -n "$target" ]] && shift
  case "$target" in
    runtime) run_internal cleanup.sh "$@" ;;
    cargo) run_internal cleanup-cargo-target.sh "$@" ;;
    *) aletheon_die "usage: aletheon.sh cleanup {runtime|cargo} [options]" || return 2 ;;
  esac
}
