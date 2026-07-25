#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
entry="$root/scripts/aletheon.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if bash "$entry" not-a-command >"$tmp/out" 2>"$tmp/err"; then
  echo 'unknown command unexpectedly succeeded' >&2
  exit 1
else
  [[ $? -eq 2 ]]
fi
grep -q 'Unknown command' "$tmp/err"

mkdir -p "$tmp/home/.aletheon"
cat >"$tmp/home/.aletheon/config.toml" <<'EOF'
[[mcp_servers]]
name = "gbrain"
url = "file:///tmp/not-http"
EOF
if HOME="$tmp/home" bash "$entry" configure check >"$tmp/out" 2>"$tmp/err"; then
  echo 'invalid endpoint unexpectedly passed' >&2
  exit 1
fi
grep -q 'invalid HTTP endpoint' "$tmp/err"

cat >"$tmp/home/.aletheon/config.toml" <<'EOF'
[[mcp_servers]]
name = "gbrain"
url = "http://100.64.0.10:3131/mcp"
EOF
HOME="$tmp/home" bash "$entry" configure check | grep -q 'configuration paths and endpoint syntax are valid'
HOME="$tmp/home" bash "$entry" configure show | grep -q 'gbrain_endpoint=http://100.64.0.10:3131/mcp'

mkdir -p "$tmp/bin" "$tmp/user-bin" "$tmp/user-units"
cat >"$tmp/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$tmp/bin/systemd-analyze" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$tmp/bin/systemctl" "$tmp/bin/systemd-analyze"
PATH="$tmp/bin:$PATH" HOME="$tmp/home" \
  ALETHEON_USER_BIN_DIR="$tmp/user-bin" ALETHEON_USER_UNIT_DIR="$tmp/user-units" \
  bash "$entry" closure install >/dev/null
cmp "$root/scripts/libexec/aletheon/pi-scheduled-task.sh" "$tmp/user-bin/aletheon-pi-scheduled-task"
cmp "$root/deploy/systemd/user/aletheon-pi-closure.service" "$tmp/user-units/aletheon-pi-closure.service"
cmp "$root/deploy/systemd/user/aletheon-pi-closure.timer" "$tmp/user-units/aletheon-pi-closure.timer"

mkdir -p "$tmp/libexec/verify"
cat >"$tmp/libexec/fake-command" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "$(basename "$0")" "$*"
EOF
chmod +x "$tmp/libexec/fake-command"
for command in backup restore upgrade cleanup cleanup-cargo-target secret-init \
  secret-audit sqlite-check architecture-check release-acceptance; do
  ln -s fake-command "$tmp/libexec/$command.sh"
done
for command in systemd network-exposure compose migration-matrix multi-user-runtime; do
  ln -s ../fake-command "$tmp/libexec/verify/$command.sh"
done

ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" backup --fixture |
  grep -q 'backup.sh|--fixture'
ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" cleanup cargo |
  grep -q 'cleanup-cargo-target.sh|'
ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" secrets init fixture-root |
  grep -q 'secret-init.sh|fixture-root'
ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" database check fixture.db |
  grep -q 'sqlite-check.sh|fixture.db'
ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" verify systemd --fixture |
  grep -q 'systemd.sh|--fixture'
ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" acceptance architecture |
  grep -q 'architecture-check.sh|'

if ALETHEON_LIBEXEC="$tmp/libexec" bash "$entry" cleanup unknown \
    >"$tmp/out" 2>"$tmp/err"; then
  echo 'unknown cleanup target unexpectedly succeeded' >&2
  exit 1
else
  [[ $? -eq 2 ]]
fi
grep -q 'usage: aletheon.sh cleanup' "$tmp/err"

echo 'operations CLI integration tests passed'
