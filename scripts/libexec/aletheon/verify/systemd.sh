#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --preflight --binary PATH --config PATH | --readiness --socket PATH [--timeout SEC] | --core-unit UNIT --binary PATH | --user-units SERVICE SOCKET --binary PATH" >&2
  exit 64
}

mode="" binary="" config="" socket="" unit="" service_unit="" socket_unit="" timeout=30
while (($#)); do
  case "$1" in
    --preflight|--readiness) mode="$1"; shift ;;
    --core-unit) mode=--core-unit; unit=${2-}; shift 2 ;;
    --user-units)
      mode=--user-units
      if (($# >= 3)) && [[ ${2-} != --* && ${3-} != --* ]]; then
        service_unit=$2; socket_unit=$3; shift 3
      else
        service_unit=config/aletheon.user.service
        socket_unit=config/aletheon.user.socket
        shift
      fi
      ;;
    --binary) binary=${2-}; shift 2 ;;
    --config) config=${2-}; shift 2 ;;
    --socket) socket=${2-}; shift 2 ;;
    --timeout) timeout=${2-}; shift 2 ;;
    *) usage ;;
  esac
done

# Repository-local verification does not need the production binary. The script
# itself is an executable, stable stand-in for systemd-analyze path validation.
if [[ "$mode" == --user-units && -z "$binary" ]]; then
  binary=$0
fi

stage_unit() {
  local source=$1 destination=$2
  local verifier scripts_dir
  verifier=$(realpath "$0")
  scripts_dir=$(dirname "$verifier")
  sed -e "s#/usr/bin/aletheon#$binary#g" \
      -e "s#%h/.local/bin/aletheon#$binary#g" \
      -e "s#/usr/libexec/aletheon/verify-systemd.sh#$verifier#g" \
      -e "s#/usr/libexec/aletheon/aletheon-secret-audit.sh#$scripts_dir/aletheon-secret-audit.sh#g" \
      -e "s#/usr/libexec/aletheon/backup-aletheon.sh#$scripts_dir/backup-aletheon.sh#g" \
      -e "s#/usr/libexec/aletheon/cleanup-aletheon.sh#$scripts_dir/cleanup-aletheon.sh#g" \
      "$source" >"$destination"
}

require_contract() {
  local file=$1 contract=$2 message=$3
  grep -Eq "$contract" "$file" || {
    echo "$message: missing boundary $contract" >&2
    exit 1
  }
}

case "$mode" in
  --core-unit)
    [[ -f "$unit" && -x "$binary" ]] || usage
    binary=$(realpath "$binary")
    staged_dir=$(mktemp -d)
    trap 'rm -rf "$staged_dir"' EXIT
    staged="$staged_dir/aletheon-core.service"
    stage_unit "$unit" "$staged"
    systemd-analyze verify "$staged"
    for contract in \
      '^User=aletheon$' '^Group=aletheon$' '^NoNewPrivileges=yes$' \
      '^ProtectSystem=strict$' '^StandardOutput=journal$' \
      '^RestrictAddressFamilies=.*AF_UNIX'; do
      require_contract "$staged" "$contract" 'core unit verification'
    done
    require_contract "$staged" \
      '^ExecStart=.* core .*--config .*--socket /run/aletheon/core\.sock$' \
      'core unit verification'
    require_contract "$staged" \
      '^ReadWritePaths=/run/aletheon /var/lib/aletheon /var/cache/aletheon$' \
      'core unit verification'
    if grep -Eq '^ReadWritePaths=.*(/home|/tmp)' "$staged"; then
      echo 'core unit verification: user or temporary writable root is forbidden' >&2
      exit 1
    fi
    ;;
  --user-units)
    [[ -f "$service_unit" && -f "$socket_unit" && -x "$binary" ]] || usage
    binary=$(realpath "$binary")
    staged_dir=$(mktemp -d)
    trap 'rm -rf "$staged_dir"' EXIT
    staged_service="$staged_dir/aletheon.service"
    staged_socket="$staged_dir/aletheon.socket"
    stage_unit "$service_unit" "$staged_service"
    stage_unit "$socket_unit" "$staged_socket"
    systemd-analyze verify "$staged_socket" "$staged_service"
    for contract in \
      '^ExecStart=.* daemon$' '^NoNewPrivileges=yes$' '^LimitCORE=0$'; do
      require_contract "$staged_service" "$contract" 'user unit verification'
    done
    if grep -Eq '^(User|Group)=' "$staged_service"; then
      echo 'user unit verification: a user-manager service must not switch identity' >&2
      exit 1
    fi
    if grep -Eq '^RuntimeDirectory=' "$staged_service"; then
      echo 'user unit verification: socket unit must exclusively own the runtime directory' >&2
      exit 1
    fi
    if grep -Eq '(/etc/aletheon/credentials|/var/lib/aletheon|/var/cache/aletheon)' "$staged_service"; then
      echo 'user unit verification: machine-scoped state or credentials leaked into user runtime' >&2
      exit 1
    fi
    for contract in \
      '^ListenStream=%t/aletheon/aletheon\.sock$' \
      '^DirectoryMode=0700$' '^SocketMode=0600$' '^WantedBy=sockets\.target$'; do
      require_contract "$staged_socket" "$contract" 'user socket verification'
    done
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
