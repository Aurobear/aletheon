#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --preflight --binary PATH --config PATH | --readiness --socket PATH [--timeout SEC] | --unit UNIT --binary PATH" >&2
  exit 64
}

mode="" binary="" config="" socket="" unit="" timeout=30
while (($#)); do
  case "$1" in
    --preflight|--readiness) mode="$1"; shift ;;
    --unit) mode=--unit; unit=${2-}; shift 2 ;;
    --binary) binary=${2-}; shift 2 ;;
    --config) config=${2-}; shift 2 ;;
    --socket) socket=${2-}; shift 2 ;;
    --timeout) timeout=${2-}; shift 2 ;;
    *) usage ;;
  esac
done

case "$mode" in
  --unit)
    [[ -f "$unit" && -x "$binary" ]] || usage
    binary=$(realpath "$binary")
    verifier=$(realpath "$0")
    scripts_dir=$(dirname "$verifier")
    staged=$(mktemp --suffix=.service)
    trap 'rm -f "$staged"' EXIT
    sed -e "s#/usr/bin/aletheon#$binary#g" \
        -e "s#/usr/libexec/aletheon/verify-systemd.sh#$verifier#g" \
        -e "s#/usr/libexec/aletheon/aletheon-secret-audit.sh#$scripts_dir/aletheon-secret-audit.sh#g" \
        -e "s#/usr/libexec/aletheon/backup-aletheon.sh#$scripts_dir/backup-aletheon.sh#g" \
        -e "s#/usr/libexec/aletheon/cleanup-aletheon.sh#$scripts_dir/cleanup-aletheon.sh#g" \
        "$unit" > "$staged"
    systemd-analyze verify "$staged"
    # Release verification is not only syntax: assert the installed unit keeps
    # the daemon unprivileged, journal-backed and limited to explicit roots.
    for contract in \
      '^User=aletheon$' '^Group=aletheon$' '^NoNewPrivileges=yes$' \
      '^ProtectSystem=strict$' '^StandardOutput=journal$' \
      '^RestrictAddressFamilies=.*AF_UNIX'; do
      grep -Eq "$contract" "$staged" || {
        echo "unit verification: missing boundary $contract" >&2; exit 1;
      }
    done
    grep -Eq '^ExecStart=.* core .*--config .*--socket /run/aletheon/core\.sock' "$staged" || {
      echo "unit verification: core must expose the explicit internal AF_UNIX socket" >&2; exit 1;
    }
    ;;
  --preflight)
    [[ -x "$binary" && -f "$config" && ! -L "$config" ]] || {
      echo "preflight: binary/config missing or unsafe" >&2; exit 1;
    }
    [[ $(stat -c '%a' "$config") =~ ^(600|640|644)$ ]] || {
      echo "preflight: config mode must be 0600/0640/0644" >&2; exit 1;
    }
    "$binary" core --help | grep -q -- '--config <CONFIG>'
    "$binary" core --help | grep -q -- '--socket <SOCKET>'
    "$binary" daemon --help | grep -q -- '--config <CONFIG>'
    "$binary" daemon --help | grep -q -- '--socket <SOCKET>'
    python3 - "$config" <<'PY'
import pathlib, sys, tomllib
path = pathlib.Path(sys.argv[1])
with path.open('rb') as stream:
    config = tomllib.load(stream)
deployment = config.get('deployment', {})
if deployment.get('mode') != 'production':
    raise SystemExit('preflight: deployment.mode must be production')
paths = deployment.get('paths', {})
expected = {
    'state_root': '/var/lib/aletheon', 'config_root': '/etc/aletheon',
    'runtime_root': '/run/aletheon', 'cache_root': '/var/cache/aletheon',
}
for key, value in expected.items():
    if paths.get(key) != value:
        raise SystemExit(f'preflight: invalid {key}')
for key, value in paths.items():
    candidate = pathlib.PurePath(value)
    if '~' in value or not candidate.is_absolute() or '..' in candidate.parts:
        raise SystemExit(f'preflight: unsafe path {key}')
PY
    ;;
  --readiness)
    [[ -n "$socket" && "$timeout" =~ ^[0-9]+$ ]] || usage
    python3 - "$socket" "$timeout" <<'PY'
import json, socket, sys, time
path, timeout = sys.argv[1], int(sys.argv[2])
client = None
deadline = time.monotonic() + timeout
request = json.dumps({'jsonrpc':'2.0','id':1,'method':'health','params':{}}).encode() + b'\n'
while time.monotonic() < deadline:
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(2)
        client.connect(path)
        client.sendall(request)
        response = b''
        while not response.endswith(b'\n'):
            chunk = client.recv(65536)
            if not chunk: break
            response += chunk
        payload = json.loads(response)
        if payload.get('result', {}).get('readiness') in ('ready', 'degraded'):
            raise SystemExit(0)
    except (OSError, ValueError, json.JSONDecodeError):
        time.sleep(.25)
    finally:
        if client is not None:
            try: client.close()
            except Exception: pass
raise SystemExit('readiness: daemon health RPC did not become ready')
PY
    ;;
  *) usage ;;
esac
