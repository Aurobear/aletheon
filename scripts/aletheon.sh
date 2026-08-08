#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
ALETHEON_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)
export SCRIPT_DIR ALETHEON_ROOT

# Build and deploy must keep the invoking user's shared Cargo cache and user
# systemd manager. Re-enter before sourcing common.sh so HOME-derived paths do
# not silently switch to root and trigger a cold rebuild.
if [[ ${1:-} == build || ${1:-} == deploy ]] &&
   [[ ${EUID:-$(id -u)} -eq 0 ]] &&
   [[ -n ${SUDO_USER:-} ]] &&
   [[ $SUDO_USER != root ]] &&
   [[ ${ALETHEON_DEPLOY_AS_USER:-0} != 1 ]]; then
  deploy_user=$SUDO_USER
  deploy_uid=$(id -u "$deploy_user")
  deploy_home=$(getent passwd "$deploy_user" | cut -d: -f6)
  [[ -n $deploy_home ]] || {
    printf 'Unable to resolve home directory for sudo user: %s\n' "$deploy_user" >&2
    exit 1
  }
  exec sudo -u "$deploy_user" -H env \
    ALETHEON_DEPLOY_AS_USER=1 \
    ALETHEON_DEPLOY_SCOPE=system \
    "HOME=$deploy_home" \
    "XDG_RUNTIME_DIR=/run/user/$deploy_uid" \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$deploy_uid/bus" \
    bash "$0" "$@"
fi

source "$SCRIPT_DIR/lib/aletheon/common.sh"
source "$SCRIPT_DIR/lib/aletheon/build.sh"
source "$SCRIPT_DIR/lib/aletheon/install.sh"
source "$SCRIPT_DIR/lib/aletheon/service.sh"
source "$SCRIPT_DIR/lib/aletheon/maintenance.sh"
source "$SCRIPT_DIR/lib/aletheon/security.sh"
source "$SCRIPT_DIR/lib/aletheon/acceptance.sh"
source "$SCRIPT_DIR/lib/aletheon/test.sh"
source "$SCRIPT_DIR/lib/aletheon/runtime_gate.sh"
source "$SCRIPT_DIR/lib/aletheon/verify.sh"

usage() {
  cat <<'EOF'
Usage: scripts/aletheon.sh <command> [options]

Deployment:
  build                         Build the release binary through cargo-agent.sh
  install [--no-enable]         Install native systemd assets
  deploy [--no-build] [--no-restart] [--no-enable]
                                Build, install, stage closure, restart, verify.
                                Scope is inferred from sudo: with sudo it
                                installs system assets, without sudo it installs
                                a rootless runtime under $HOME.
  configure {show|check}        Display safe paths or validate configuration

Operations:
  status                        Show and validate service/timer state
  health                        Probe core, user daemon, and GBrain
  restart                       Restart core and user daemon
  logs [core|user|closure]      Show recent journal entries
  backup | restore | upgrade    Run production lifecycle operations
  cleanup {runtime|cargo}       Clean managed runtime or Cargo state
  secrets {init|audit}          Initialize or audit production credentials
  database check DATABASE...    Run read-only SQLite quick checks
  verify [TARGET]               Run deployed-state or specialized verification
  acceptance {architecture|release|extension|robot-r8}
                                Run architecture, release, or installed Robot R8 acceptance
  test {unit|operations|deployment|architecture|all}
                                Run a focused test suite
  closure {install|run|status}  Manage the scheduled Pi-memory closure
  completion {bash|zsh}         Print shell completion definitions
  help                          Show this help
EOF
}

cmd_completion() {
  local shell=${1:-}
  case "$shell" in
    bash|zsh) cat "$SCRIPT_DIR/completions/aletheon-ops.$shell" ;;
    *) aletheon_die "usage: aletheon.sh completion {bash|zsh}" || return 2 ;;
  esac
}

cmd_deploy() {
  local deploy_args=("$@")
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
  # A deployment changes the installed binary and daemon generation. Hold an
  # exclusive machine-user lease for the complete build/install/restart/verify
  # transaction so installed acceptance suites can hold the matching shared
  # lease and cannot be silently invalidated midway through a run. Keep the
  # descriptor in flock's supervisor and close it in the deployed command;
  # otherwise a persistent child such as the sccache server can inherit the
  # exclusive lock and block all later acceptance runs indefinitely.
  local runtime_lock=${ALETHEON_RUNTIME_LOCK_FILE:-${XDG_RUNTIME_DIR:-$HOME/.local/state}/aletheon/runtime-mutation.lock}
  install -d -m 0700 "$(dirname -- "$runtime_lock")"
  if [[ ${ALETHEON_RUNTIME_LOCK_GUARD:-0} != 1 ]]; then
    exec flock -x -o "$runtime_lock" env \
      ALETHEON_RUNTIME_LOCK_GUARD=1 \
      bash "$SCRIPT_DIR/aletheon.sh" deploy "${deploy_args[@]}"
  fi
  unset ALETHEON_RUNTIME_LOCK_GUARD
  local install_args=()
  ((enable)) || install_args+=(--no-enable)
  ((build)) && cmd_build
  # Scope follows the invocation boundary, not sudo availability:
  # `sudo ... deploy` installs system assets, while an ordinary invocation
  # remains rootless even when passwordless sudo is configured.
  if [[ ${EUID:-$(id -u)} -eq 0 ]] ||
     [[ ${ALETHEON_DEPLOY_SCOPE:-user} == system ]]; then
    aletheon_info "sudo deployment requested; installing system assets"
    cmd_install "${install_args[@]}"
    cmd_closure_install
    ((restart)) && cmd_restart
    cmd_verify
  else
    aletheon_info "no sudo; installing rootless runtime under \$HOME"
    cmd_install_user "${install_args[@]}"
    cmd_closure_install
    ((restart)) && cmd_restart_user
    cmd_verify_user
  fi
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
  backup) shift; cmd_backup "$@" ;;
  restore) shift; cmd_restore "$@" ;;
  upgrade) shift; cmd_upgrade "$@" ;;
  cleanup) shift; cmd_cleanup "$@" ;;
  secrets) shift; cmd_secrets "$@" ;;
  database) shift; cmd_database "$@" ;;
  verify)
    shift
    if (($#)); then cmd_verify_specialized "$@"; else cmd_verify; fi
    ;;
  acceptance) shift; cmd_acceptance "$@" ;;
  test) shift; cmd_test "$@" ;;
  closure) shift; cmd_closure "$@" ;;
  completion) shift; cmd_completion "$@" ;;
  help|--help|-h) usage ;;
  *) printf 'Unknown command: %s\n' "$1" >&2; usage >&2; exit 2 ;;
esac
