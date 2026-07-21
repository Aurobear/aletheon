#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
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
cmp "$root/scripts/aletheon-pi-scheduled-task.sh" "$tmp/user-bin/aletheon-pi-scheduled-task"
cmp "$root/deploy/systemd/user/aletheon-pi-closure.service" "$tmp/user-units/aletheon-pi-closure.service"
cmp "$root/deploy/systemd/user/aletheon-pi-closure.timer" "$tmp/user-units/aletheon-pi-closure.timer"

echo 'operations CLI integration tests passed'
