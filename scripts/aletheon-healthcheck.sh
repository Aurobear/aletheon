#!/usr/bin/env bash
set -euo pipefail
socket=${1:-/run/aletheon/aletheon.sock}
python3 - "$socket" <<'PY'
import json, socket, sys
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(3)
try:
    client.connect(sys.argv[1])
    request = {'jsonrpc':'2.0','id':1,'method':'health','params':{}}
    client.sendall(json.dumps(request).encode() + b'\n')
    response = b''
    while not response.endswith(b'\n'):
        chunk = client.recv(65536)
        if not chunk: break
        response += chunk
    result = json.loads(response).get('result', {})
    readiness = result.get('readiness')
    print(json.dumps({
        'liveness': result.get('liveness'),
        'readiness': readiness,
        'components': result.get('components', {}),
    }, sort_keys=True, separators=(',', ':')))
    raise SystemExit({'ready': 0, 'degraded': 1, 'unready': 2}.get(readiness, 2))
except (OSError, ValueError, json.JSONDecodeError) as error:
    print('healthcheck: local daemon unavailable', file=sys.stderr)
    raise SystemExit(2) from error
finally:
    client.close()
PY
