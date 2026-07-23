#!/usr/bin/env bash

_run_test_script() {
  local relative=$1
  shift
  bash "$ALETHEON_ROOT/tests/$relative" "$@"
}

cmd_test() {
  local suite=${1:-}
  [[ -n "$suite" ]] && shift
  case "$suite" in
    operations)
      _run_test_script suites/operations/script_surface_test.sh
      _run_test_script suites/operations/cli_static_test.sh
      _run_test_script suites/operations/cli_test.sh
      _run_test_script suites/operations/installed_runtime_gate_test.sh
      _run_test_script suites/operations/completion_test.sh
      ;;
    architecture)
      _run_test_script suites/architecture/architecture_check.sh
      _run_test_script suites/architecture/path_inventory.sh
      run_internal architecture-check.sh
      ;;
    deployment)
      _run_test_script suites/deployment/systemd_runtime_boundary.sh
      _run_test_script suites/deployment/upgrade_multi_user_test.sh
      _run_test_script production/installed_host_static_test.sh
      _run_test_script production/failure_matrix_static_test.sh
      _run_test_script production/release_aggregate_receipt_test.sh
      ;;
    unit)
      bash "$ALETHEON_ROOT/scripts/cargo-agent.sh" test --workspace "$@"
      ;;
    all)
      cmd_test operations
      cmd_test architecture
      cmd_test deployment
      cmd_test unit "$@"
      ;;
    *) aletheon_die "usage: aletheon.sh test {unit|operations|deployment|architecture|all}" || return 2 ;;
  esac
}
