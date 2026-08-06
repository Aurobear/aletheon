#!/usr/bin/env bash

cmd_acceptance() {
  local lane=${1:-}
  [[ -n "$lane" ]] && shift
  case "$lane" in
    architecture) run_internal architecture-check.sh "$@" ;;
    release) run_internal release-acceptance.sh "$@" ;;
    extension) run_internal extension-acceptance.sh "$@" ;;
    robot-r8)
      # Report/SQLite evidence is only one half of R8. Refuse to emit an
      # acceptance result unless the system-installed binaries, live daemon
      # executables, restart counters, Memory Agent and official socket have
      # first passed the canonical installed-runtime gate.
      cmd_installed_runtime_gate || return
      run_internal robot_r8_evidence.py "$@"
      ;;
    *) aletheon_die "usage: aletheon.sh acceptance {architecture|release|extension|robot-r8} [options]" || return 2 ;;
  esac
}
