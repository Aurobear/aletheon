#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home/.cargo/bin" "$tmp/target"

cat >"$tmp/home/.cargo/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$FAKE_CARGO_LOG"
printf 'wrapper=%s ignore_server_io=%s\n' \
  "${RUSTC_WRAPPER:-unset}" "${SCCACHE_IGNORE_SERVER_IO_ERROR:-unset}" \
  >>"$FAKE_CARGO_ENV_LOG"
EOF
cat >"$tmp/home/.cargo/bin/sccache" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$tmp/home/.cargo/bin/du" <<'EOF'
#!/usr/bin/env bash
printf 'du %s\n' "$*" >>"$FAKE_DU_LOG"
printf '%s\t%s\n' "$FAKE_DU_KIB" "${@: -1}"
EOF
chmod +x "$tmp/home/.cargo/bin/cargo" "$tmp/home/.cargo/bin/du" \
  "$tmp/home/.cargo/bin/sccache"

export HOME="$tmp/home"
export ALETHEON_CARGO_CACHE_ROOT="$tmp/cache"
export CARGO_TARGET_DIR="$tmp/target"
export FAKE_CARGO_LOG="$tmp/cargo.log"
export FAKE_CARGO_ENV_LOG="$tmp/cargo-env.log"
export FAKE_DU_LOG="$tmp/du.log"
export FAKE_DU_KIB=1024
export ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC=900

# The first invocation establishes the measurement stamp; subsequent hot
# commands must not recursively scan the target again inside the build lock.
bash "$root/scripts/cargo-agent.sh" check -p aletheon >/dev/null
bash "$root/scripts/cargo-agent.sh" check -p aletheon >/dev/null
[[ $(wc -l <"$FAKE_DU_LOG") -eq 1 ]]
[[ $(grep -c '^wrapper=sccache ignore_server_io=1$' "$FAKE_CARGO_ENV_LOG") -eq 2 ]]

# An explicit failover policy remains authoritative.
SCCACHE_IGNORE_SERVER_IO_ERROR=0 \
  bash "$root/scripts/cargo-agent.sh" check -p aletheon >/dev/null
grep -q '^wrapper=sccache ignore_server_io=0$' "$FAKE_CARGO_ENV_LOG"

ALETHEON_CARGO_TARGET_FORCE_SCAN=1 \
  bash "$root/scripts/cargo-agent.sh" check -p aletheon >/dev/null
[[ $(wc -l <"$FAKE_DU_LOG") -eq 2 ]]

# An explicit audit retains the bounded-cache cleanup behavior.
FAKE_DU_KIB=$((2 * 1024 * 1024)) \
  ALETHEON_CARGO_TARGET_MAX_GIB=1 \
  ALETHEON_CARGO_TARGET_FORCE_SCAN=1 \
  bash "$root/scripts/cargo-agent.sh" check -p aletheon >/dev/null
[[ $(grep -c '^cargo clean --target-dir ' "$FAKE_CARGO_LOG") -eq 1 ]]

if ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC=invalid \
  bash "$root/scripts/cargo-agent.sh" check >/dev/null 2>&1; then
  echo "invalid target scan interval was accepted" >&2
  exit 1
fi

echo "cargo agent cache scan: pass"
