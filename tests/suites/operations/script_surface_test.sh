#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
cd "$root"

mapfile -t public_scripts < <(find scripts -maxdepth 1 -type f -name '*.sh' -printf '%f\n' | sort)
expected=(aletheon.sh cargo-agent.sh)
[[ "${public_scripts[*]}" == "${expected[*]}" ]] || {
  printf 'unexpected public scripts: %s\n' "${public_scripts[*]}" >&2
  exit 1
}
[[ -x setup.sh && -x scripts/aletheon.sh && -x scripts/cargo-agent.sh ]]
! grep -Eq '(^|[[:space:]])cargo (build|check|test|clippy|doc)' setup.sh
grep -Fq 'scripts/cargo-agent.sh build -p aletheon --release' setup.sh

removed=(
  scripts/aletheon-healthcheck.sh
  scripts/aletheon-pi-scheduled-task.sh
  scripts/aletheon-secret-audit.sh
  scripts/aletheon-secret-init.sh
  scripts/aletheon-sqlite-check.sh
  scripts/architecture-check.sh
  scripts/backup-aletheon.sh
  scripts/cleanup-aletheon.sh
  scripts/cleanup-cargo-target.sh
  scripts/install-systemd.sh
  scripts/release-acceptance.sh
  scripts/restore-aletheon.sh
  scripts/upgrade-aletheon.sh
  scripts/verify-compose.sh
  scripts/verify-migration-matrix.sh
  scripts/verify-multi-user-runtime.sh
  scripts/verify-network-exposure.sh
  scripts/verify-systemd.sh
)
live_paths=(.github config crates deploy docs/deployment docs/design docs/testing scripts tests
  architecture-status.toml justfile setup.sh)
for path in "${removed[@]}"; do
  output=$(git grep -n -F "$path" -- "${live_paths[@]}" || true)
  output=$(grep -v '^tests/suites/operations/script_surface_test.sh:' <<<"$output" || true)
  if [[ -n "$output" ]]; then
    printf '%s\n' "$output" >&2
    echo "removed script path is still referenced: $path" >&2
    exit 1
  fi
done

help=$(bash scripts/aletheon.sh help)
for command in backup restore upgrade cleanup secrets database verify acceptance test completion; do
  grep -q "$command" <<<"$help"
done

echo 'script public surface: pass'
