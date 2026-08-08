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

# Fresh installs must not silently select or advertise the quota-expensive Pro
# route. Pro may remain in the model catalog for explicit user selection, but
# every generated default and convenience alias is Flash-only.
setup_config=$(sed -n '/^setup_config()/,/^}/p' setup.sh)
grep -Fq 'default_model = "deepseek/deepseek-v4-flash"' <<<"$setup_config"
grep -Fq 'models = ["deepseek/deepseek-v4-flash"]' <<<"$setup_config"
grep -Fq 'models = ["deepseek-v4-flash"]' <<<"$setup_config"
grep -Fq 'flash = "leju/deepseek/deepseek-v4-flash"' <<<"$setup_config"
grep -Fq 'deepseek = "deepseek/deepseek-v4-flash"' <<<"$setup_config"
if grep -Fq 'deepseek-v4-pro' <<<"$setup_config"; then
  echo 'fresh-install defaults must not contain a DeepSeek Pro route' >&2
  exit 1
fi
for config in config/default.toml config/production.toml.example; do
  grep -Fq 'deepseek-v4-flash[1m]' "$config"
  if grep -Fq 'deepseek-v4-pro' "$config"; then
    echo "configured defaults must not contain a DeepSeek Pro route: $config" >&2
    exit 1
  fi
done

# Every local edit/verify profile preserves rustc incremental state. Only the
# tagged distributable release job may opt out for its clean artifact lane.
python3 - "$root/Cargo.toml" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as source:
    manifest = tomllib.load(source)
for name in ("dev", "test", "release", "bench"):
    if manifest.get("profile", {}).get(name, {}).get("incremental") is not True:
        raise SystemExit(f"Cargo profile {name} must enable incremental compilation")
PY
if grep -R -n -F 'CARGO_INCREMENTAL=0' scripts setup.sh justfile; then
  echo 'local build/test/deployment paths must not disable incremental compilation' >&2
  exit 1
fi
grep -Fq 'CARGO_INCREMENTAL: 0' .github/workflows/release.yml

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
grep -Fq 'robot-r8' <<<"$help"

echo 'script public surface: pass'
