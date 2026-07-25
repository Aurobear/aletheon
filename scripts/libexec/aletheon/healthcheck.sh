#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage:
  aletheon-healthcheck.sh [--user-socket PATH]
  aletheon-healthcheck.sh --authorized-users FILE
  aletheon-healthcheck.sh --core-socket PATH

A positional socket path remains accepted as a legacy alias for --user-socket.
EOF
  exit 64
}

mode=user
target=/run/aletheon/aletheon.sock
case $# in
  0) ;;
  1)
    case "$1" in
      --*) usage ;;
      *) target=$1 ;;
    esac
    ;;
  2)
    case "$1" in
      --user-socket) mode=user; target=$2 ;;
      --authorized-users) mode=users; target=$2 ;;
      --core-socket) mode=core; target=$2 ;;
      *) usage ;;
    esac
    ;;
  *) usage ;;
esac

python3 - "$mode" "$target" <<'PY'
import grp
import json
import os
import pwd
import socket
import stat
import sys

mode, target = sys.argv[1:]


def fail(message, code=2):
    print(f"healthcheck: {message}", file=sys.stderr)
    raise SystemExit(code)


def socket_stat(path, expected_uid=None, expected_gid=None, expected_mode=None):
    try:
        metadata = os.lstat(path)
    except OSError as error:
        fail(f"socket is unavailable: {path}: {error}")
    if not stat.S_ISSOCK(metadata.st_mode):
        fail(f"path is not an AF_UNIX socket: {path}")
    if expected_uid is not None and metadata.st_uid != expected_uid:
        fail(f"socket has unexpected owner: {path}")
    if expected_gid is not None and metadata.st_gid != expected_gid:
        fail(f"socket has unexpected group: {path}")
    if expected_mode is not None and stat.S_IMODE(metadata.st_mode) != expected_mode:
        fail(f"socket has unsafe mode: {path}")


def daemon_health(path, expected_uid=None, expected_gid=None):
    socket_stat(path, expected_uid=expected_uid, expected_gid=expected_gid,
                expected_mode=0o600 if expected_uid is not None else None)
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(3)
    try:
        client.connect(path)
        request = {"jsonrpc": "2.0", "id": 1, "method": "health", "params": {}}
        client.sendall(json.dumps(request).encode() + b"\n")
        response = b""
        while not response.endswith(b"\n"):
            chunk = client.recv(65536)
            if not chunk:
                break
            response += chunk
        envelope = json.loads(response)
        if envelope.get("id") != 1 or "error" in envelope:
            fail(f"invalid health response from {path}")
        result = envelope.get("result", {})
        readiness = result.get("readiness")
        report = {
            "liveness": result.get("liveness"),
            "readiness": readiness,
            "components": result.get("components", {}),
        }
        return report, {"ready": 0, "degraded": 1, "unready": 2}.get(readiness, 2)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        fail(f"local daemon unavailable: {path}: {error}")
    finally:
        client.close()


def authorized_users(path):
    try:
        metadata = os.lstat(path)
    except OSError as error:
        fail(f"authorized-user manifest is unavailable: {error}")
    if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        fail("authorized-user manifest must be a regular non-symlink file")
    if os.geteuid() == 0 and metadata.st_uid != 0:
        fail("authorized-user manifest must be root-owned")
    if stat.S_IMODE(metadata.st_mode) & 0o022:
        fail("authorized-user manifest must not be group/world writable")
    seen = set()
    users = []
    try:
        with open(path, encoding="utf-8") as source:
            for line_number, raw in enumerate(source, 1):
                value = raw.split("#", 1)[0].strip()
                if not value:
                    continue
                if any(character.isspace() for character in value) or "/" in value:
                    fail(f"invalid user at manifest line {line_number}")
                if value in seen:
                    fail(f"duplicate authorized user: {value}")
                try:
                    principal = pwd.getpwnam(value)
                except KeyError:
                    fail(f"authorized user does not exist: {value}")
                if principal.pw_uid == 0 or value == "aletheon":
                    fail(f"refusing privileged/service principal: {value}")
                seen.add(value)
                users.append(principal)
    except UnicodeError as error:
        fail(f"authorized-user manifest is not UTF-8: {error}")
    if not users:
        fail("authorized-user manifest is empty")
    return users


if mode == "core":
    try:
        core_group = grp.getgrnam("aletheon").gr_gid
    except KeyError:
        fail("aletheon group does not exist")
    socket_stat(target, expected_gid=core_group, expected_mode=0o660)
    print(json.dumps({"core_socket": target, "readiness": "ready"}, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)

if mode == "user":
    report, status = daemon_health(target)
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    raise SystemExit(status)

users = authorized_users(target)
reports = []
status = 0
for principal in users:
    path = f"/run/user/{principal.pw_uid}/aletheon/aletheon.sock"
    report, user_status = daemon_health(path, expected_uid=principal.pw_uid,
                                        expected_gid=principal.pw_gid)
    reports.append({"user": principal.pw_name, "uid": principal.pw_uid, "socket": path, **report})
    status = max(status, user_status)
print(json.dumps({"authorized_users": reports, "readiness": "ready" if status == 0 else "degraded"},
                 sort_keys=True, separators=(",", ":")))
raise SystemExit(status)
PY
