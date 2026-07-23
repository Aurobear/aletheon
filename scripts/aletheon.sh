#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
ALETHEON_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)
export SCRIPT_DIR ALETHEON_ROOT

source "$SCRIPT_DIR/lib/aletheon/common.sh"
source "$SCRIPT_DIR/lib/aletheon/build.sh"
source "$SCRIPT_DIR/lib/aletheon/install.sh"
source "$SCRIPT_DIR/lib/aletheon/service.sh"
source "$SCRIPT_DIR/lib/aletheon/runtime_gate.sh"
source "$SCRIPT_DIR/lib/aletheon/verify.sh"

usage() {
  cat <<'EOF'
Usage: scripts/aletheon.sh <command> [options]

Deployment:
  build                         Build the release binary through cargo-agent.sh
  install [--no-enable]         Install native systemd assets
  deploy [--no-build] [--no-restart] [--no-enable]
                                Build, install, stage closure, restart, verify
  configure {show|check}        Display safe paths or validate configuration

Operations:
  status                        Show and validate service/timer state
  health                        Probe core, user daemon, and GBrain
  restart                       Restart core and user daemon
  logs [core|user|closure]      Show recent journal entries
  verify                        Run the complete deployed-state gate
  closure {install|run|status}  Manage the scheduled Pi-memory closure
  help                          Show this help
EOF
}

cmd_deploy() {
  local build=1 restart=1 enable=1
  while (($#)); do
    case "$1" in
      --no-build) build=0 ;;
      --no-restart) restart=0 ;;
      --no-enable) enable=0 ;;
      *) aletheon_die "unknown deploy option: $1"; return 2 ;;
    esac
    shift
  done
  ((build)) && cmd_build
  if ((enable)); then cmd_install; else cmd_install --no-enable; fi
  cmd_closure_install
  ((restart)) && cmd_restart
  cmd_verify
}

case "${1:-help}" in
  build) shift; cmd_build "$@" ;;
  install) shift; cmd_install "$@" ;;
  deploy) shift; cmd_deploy "$@" ;;
  configure) shift; cmd_configure "$@" ;;
  status) shift; cmd_status "$@" ;;
  health) shift; cmd_health "$@" ;;
  restart) shift; cmd_restart "$@" ;;
  logs) shift; cmd_logs "$@" ;;
  verify) shift; cmd_verify "$@" ;;
  closure) shift; cmd_closure "$@" ;;
  help|--help|-h) usage ;;
  *) printf 'Unknown command: %s\n' "$1" >&2; usage >&2; exit 2 ;;
esac
