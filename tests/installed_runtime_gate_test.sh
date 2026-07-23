#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/candidate" "$tmp/installed" "$tmp/runtime" \
  "$tmp/proc/101" "$tmp/proc/202" "$tmp/state"

cat >"$tmp/candidate/aletheon" <<'EOF'
#!/usr/bin/env bash
case "${FAKE_CLIENT_MODE:-success}" in
  success) printf 'ALETHEON_DEPLOYMENT_OK\n' ;;
  empty) exit 0 ;;
  fail) exit 7 ;;
esac
EOF
chmod +x "$tmp/candidate/aletheon"
cp "$tmp/candidate/aletheon" "$tmp/installed/aletheon"
cp "$tmp/candidate/aletheon" "$tmp/runtime/core"
cp "$tmp/candidate/aletheon" "$tmp/runtime/user"
ln -s "$tmp/installed/aletheon" "$tmp/proc/101/exe"
ln -s "$tmp/installed/aletheon" "$tmp/proc/202/exe"
printf '0\n' >"$tmp/state/core.restarts"
printf '0\n' >"$tmp/state/user.restarts"

cat >"$tmp/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
scope=core
if [[ ${1:-} == --user ]]; then scope=user; shift; fi
[[ ${1:-} == show ]] || exit 0
shift
unit=${1:-}
shift
property=
for arg in "$@"; do
  case "$arg" in --property=*) property=${arg#--property=} ;; esac
done
case "$property" in
  MainPID) [[ "$scope" == core ]] && echo 101 || echo 202 ;;
  ActiveState) echo active ;;
  NRestarts) cat "$FIXTURE_STATE/$scope.restarts" ;;
  *) echo "unsupported property: $property for $unit" >&2; exit 2 ;;
esac
EOF

cat >"$tmp/bin/journalctl" <<'EOF'
#!/usr/bin/env bash
[[ -n ${FAKE_FATAL_LOG:-} ]] && printf '%s\n' "$FAKE_FATAL_LOG"
exit 0
EOF

cat >"$tmp/bin/sleep" <<'EOF'
#!/usr/bin/env bash
if [[ ${FAKE_SLEEP_ACTION:-} == restart-user ]]; then
  value=$(cat "$FIXTURE_STATE/user.restarts")
  printf '%s\n' "$((value + 1))" >"$FIXTURE_STATE/user.restarts"
fi
exit 0
EOF

cat >"$tmp/bin/timeout" <<'EOF'
#!/usr/bin/env bash
shift
exec "$@"
EOF
chmod +x "$tmp/bin/"*

export PATH="$tmp/bin:$PATH"
export FIXTURE_STATE="$tmp/state"
export ALETHEON_ROOT="$root"
export ALETHEON_RELEASE_BINARY="$tmp/candidate/aletheon"
export ALETHEON_INSTALLED_BINARY="$tmp/installed/aletheon"
export ALETHEON_PROC_ROOT="$tmp/proc"
export ALETHEON_STABILITY_SECONDS=0
export ALETHEON_SMOKE_TIMEOUT_SECONDS=2
export ALETHEON_USER_SOCKET="$tmp/aletheon.sock"
export ALETHEON_CORE_UNIT=aletheon-core.service
export ALETHEON_USER_UNIT=aletheon.service

source "$root/scripts/lib/aletheon/common.sh"
source "$root/scripts/lib/aletheon/runtime_gate.sh"

run_failure() {
  local expected=$1
  shift
  if "$@" >"$tmp/out" 2>"$tmp/err"; then
    echo "expected failure containing: $expected" >&2
    exit 1
  fi
  grep -Fq "$expected" "$tmp/err"
}

case ${1:-all} in
  all|provenance)
    cmd_verify_runtime_provenance >"$tmp/out"
    grep -Fq 'installed runtime provenance verified' "$tmp/out"

    printf '\n# mismatch\n' >>"$tmp/installed/aletheon"
    run_failure 'installed binary hash differs from release candidate' \
      cmd_verify_runtime_provenance
    run_failure 'installed binary hash differs from release candidate' \
      cmd_installed_runtime_gate
    cp "$tmp/candidate/aletheon" "$tmp/installed/aletheon"

    printf '\n# mismatch\n' >>"$tmp/runtime/user"
    ln -sfn "$tmp/runtime/user" "$tmp/proc/202/exe"
    run_failure 'running executable hash differs from release candidate' \
      cmd_verify_runtime_provenance
    cp "$tmp/candidate/aletheon" "$tmp/runtime/user"
    run_failure 'running executable path differs from installed binary' \
      cmd_verify_runtime_provenance
    ln -sfn "$tmp/installed/aletheon" "$tmp/proc/202/exe"
    ;;
esac

case ${1:-all} in
  all|stability)
    cmd_verify_runtime_stability >"$tmp/out"
    grep -Fq 'runtime stability verified' "$tmp/out"

    export FAKE_SLEEP_ACTION=restart-user
    run_failure 'service restart count increased during stability window' \
      cmd_verify_runtime_stability
    unset FAKE_SLEEP_ACTION
    printf '0\n' >"$tmp/state/user.restarts"

    export FAKE_FATAL_LOG="Error: Agent profile 'robot-agent' references unknown tool 'robot_observe'"
    run_failure 'fatal startup validation error detected' \
      cmd_verify_runtime_stability
    unset FAKE_FATAL_LOG
    ;;
esac

case ${1:-all} in
  all|smoke)
    cmd_verify_official_client >"$tmp/out"
    grep -Fq 'official client real-request smoke test passed' "$tmp/out"

    export FAKE_CLIENT_MODE=fail
    run_failure 'official client real-request smoke test failed' \
      cmd_verify_official_client
    export FAKE_CLIENT_MODE=empty
    run_failure 'official client real-request returned empty output' \
      cmd_verify_official_client
    unset FAKE_CLIENT_MODE
    ;;
esac

if [[ ${1:-all} == all ]]; then
  cmd_installed_runtime_gate >"$tmp/out"
  grep -Fq 'official client real-request smoke test passed' "$tmp/out"
fi

echo 'installed runtime gate tests passed'
