#!/usr/bin/env bash
set -euo pipefail

: "${ALETHEON_TEST_USER_A:?set an existing unprivileged user}"
: "${ALETHEON_TEST_USER_B:?set a second existing unprivileged user}"

if [[ $EUID -ne 0 ]]; then
  echo 'verification requires root so runuser can enter both existing accounts' >&2
  exit 2
fi
if [[ $ALETHEON_TEST_USER_A == "$ALETHEON_TEST_USER_B" ]]; then
  echo 'the two verification users must be distinct' >&2
  exit 2
fi

for command in runuser systemctl stat python3 install; do
  command -v "$command" >/dev/null || {
    echo "missing prerequisite: $command" >&2
    exit 2
  }
done

user_env() {
  local user=$1 uid
  shift
  uid=$(id -u "$user")
  runuser -u "$user" -- env \
    "XDG_RUNTIME_DIR=/run/user/$uid" \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus" \
    "$@"
}

for user in "$ALETHEON_TEST_USER_A" "$ALETHEON_TEST_USER_B"; do
  uid=$(id -u "$user")
  gid=$(id -g "$user")
  test -d "/run/user/$uid" || {
    echo "missing prerequisite: user manager runtime for $user" >&2
    exit 2
  }
  user_env "$user" systemctl --user show-environment >/dev/null || {
    echo "missing prerequisite: running user manager for $user" >&2
    exit 2
  }
  user_env "$user" systemctl --user start aletheon.socket
  socket="/run/user/$uid/aletheon/aletheon.sock"
  test -S "$socket"
  test "$(stat -c %a "$socket")" = 600
  test "$(stat -c %u "$socket")" = "$uid"
  test "$(stat -c %g "$socket")" = "$gid"
done

uid_a=$(id -u "$ALETHEON_TEST_USER_A")
uid_b=$(id -u "$ALETHEON_TEST_USER_B")
socket_a="/run/user/$uid_a/aletheon/aletheon.sock"
socket_b="/run/user/$uid_b/aletheon/aletheon.sock"
test "$(stat -c %u "$socket_a")" = "$uid_a"
test "$(stat -c %u "$socket_b")" = "$uid_b"

# A private socket must reject the other local account before any JSON frame is
# processed. Python is used only as a Unix-socket client; no credentials are
# embedded in this verifier.
if user_env "$ALETHEON_TEST_USER_A" python3 - "$socket_b" <<'PY'
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
PY
then
  echo 'cross-user connection unexpectedly succeeded' >&2
  exit 1
fi

rpc() {
  local user=$1 socket=$2 method=$3 params=$4
  user_env "$user" python3 - "$socket" "$method" "$params" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
request = {"jsonrpc":"2.0", "id":1, "method":sys.argv[2], "params":json.loads(sys.argv[3])}
s.sendall(json.dumps(request).encode() + b"\n")
data = b""
while b"\n" not in data:
    part = s.recv(65536)
    if not part: break
    data += part
print(data.splitlines()[0].decode())
PY
}

list_a=$(rpc "$ALETHEON_TEST_USER_A" "$socket_a" approval.list '{}')
list_b=$(rpc "$ALETHEON_TEST_USER_B" "$socket_b" approval.list '{}')
python3 - "$list_a" "$list_b" <<'PY'
import json, sys
def ids(raw):
    value = json.loads(raw)
    return {item["id"] for item in value.get("result", {}).get("approvals", [])}
assert ids(sys.argv[1]).isdisjoint(ids(sys.argv[2])), "approval IDs leaked across users"
PY

fixture_root=$(mktemp -d /tmp/aletheon-multi-user.XXXXXX)
trap 'rm -rf "$fixture_root"' EXIT
for user in "$ALETHEON_TEST_USER_A" "$ALETHEON_TEST_USER_B"; do
  uid=$(id -u "$user")
  gid=$(id -g "$user")
  workspace="$fixture_root/$user"
  install -d -o "$uid" -g "$gid" -m 700 "$workspace"
  user_env "$user" aletheon -C "$workspace" exec --max-turns 4 \
    --prompt 'Use file_write to create ownership-probe.txt containing ok, then stop.' >/dev/null
  file="$workspace/ownership-probe.txt"
  test -f "$file"
  test "$(stat -c %u "$file")" = "$uid"
  test "$(stat -c %g "$file")" = "$gid"
done

echo 'multi-user runtime verification: pass'
