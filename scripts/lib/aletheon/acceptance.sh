#!/usr/bin/env bash

cmd_acceptance() {
  local lane=${1:-}
  [[ -n "$lane" ]] && shift
  case "$lane" in
    architecture) run_internal architecture-check.sh "$@" ;;
    release) run_internal release-acceptance.sh "$@" ;;
    extension) run_internal extension-acceptance.sh "$@" ;;
    *) aletheon_die "usage: aletheon.sh acceptance {architecture|release|extension} [options]" || return 2 ;;
  esac
}
