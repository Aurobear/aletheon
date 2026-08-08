#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home/.cargo/bin"

cat >"$tmp/home/.cargo/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "$CARGO_TARGET_DIR" "$*" >>"$FAKE_CARGO_LOG"
if [[ $* == *'build -p aletheon --release'* ]]; then
  install -d "$CARGO_TARGET_DIR/release"
  printf 'shared-incremental-candidate\n' >"$CARGO_TARGET_DIR/release/aletheon"
  chmod 0755 "$CARGO_TARGET_DIR/release/aletheon"
fi
EOF
chmod 0755 "$tmp/home/.cargo/bin/cargo"

HOME="$tmp/home" \
ALETHEON_CARGO_CACHE_ROOT="$tmp/cache" \
ALETHEON_RELEASE_BINARY="$tmp/staged/aletheon" \
FAKE_CARGO_LOG="$tmp/cargo.log" \
  bash "$root/scripts/aletheon.sh" build >/dev/null

cmp "$tmp/cache/target/release/aletheon" "$tmp/staged/aletheon"
grep -Fq "$tmp/cache/target|build -p aletheon --release" "$tmp/cargo.log"

echo 'shared incremental build staging: pass'
