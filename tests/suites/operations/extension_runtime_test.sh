#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
tmp=$(mktemp -d)
server_pid=
cleanup() {
  [[ -z "$server_pid" ]] || kill "$server_pid" 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

cache_root=${ALETHEON_CARGO_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aletheon-cargo}
target_dir=${CARGO_TARGET_DIR:-$cache_root/target}
binary=$target_dir/debug/aletheon
if [[ ! -x "$binary" ]]; then
  bash "$root/scripts/cargo-agent.sh" build -p aletheon --bin aletheon
fi
[[ -x "$binary" ]]

build_package() {
  local version=$1 output=$2 runtime_exit=${3:-0}
  local source="$tmp/source-$version-$(basename "$output")"
  mkdir -p "$source/assets/skills/review" "$source/assets/hooks" \
    "$source/assets/agents" "$source/assets/connectors" \
    "$source/assets/executables" "$source/payload"
  cat >"$source/extension.toml" <<MANIFEST
schema_version = 1
[package]
id = "test.full"
version = "$version"
description = "all extension asset kinds"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "skill"
id = "skill.review"
path = "assets/skills/review/SKILL.md"
[[assets]]
kind = "hook"
id = "hook.audit"
path = "assets/hooks/audit.toml"
[[assets]]
kind = "agent_profile"
id = "profile.reviewer"
path = "assets/agents/reviewer.md"
[[assets]]
kind = "connector"
id = "connector.search"
path = "assets/connectors/search.json"
[[assets]]
kind = "executable"
id = "runtime.worker"
path = "assets/executables/runtime.toml"
[requested_permissions]
executables = true
MANIFEST
  cat >"$source/assets/skills/review/SKILL.md" <<'SKILL'
---
name: test:review
description: Test review skill
---
# Review
SKILL
  cat >"$source/assets/hooks/audit.toml" <<'HOOK'
[hook]
name = "audit"
point = "PostTool"
priority = 10
script = "payload/hook.sh"
HOOK
  cat >"$source/assets/agents/reviewer.md" <<'PROFILE'
---
name: test:reviewer
description: Test reviewer profile
tools: [skill_list, skill_get]
---
Review the result.
PROFILE
  cat >"$source/assets/connectors/search.json" <<'CONNECTOR'
{"schema_version":1,"id":"test-search","transport":{"kind":"streamable_http","url":"http://127.0.0.1:9/mcp"},"request_timeout_ms":1000,"allowed_tools":[],"allowed_resources":[]}
CONNECTOR
  cat >"$source/assets/executables/runtime.toml" <<'RUNTIME'
schema_version = 1
id = "runtime.test-worker"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "payload/runtime"
[isolation]
network = false
filesystem = []
cpu_time_seconds = 5
memory_bytes = 67108864
max_processes = 2
[[capabilities]]
id = "agent.test-worker"
kind = "agent_runtime_provider"
risk = "Sandboxed"
RUNTIME
  printf '#!/usr/bin/env bash\nexit %s\n' "$runtime_exit" >"$source/payload/runtime"
  printf '#!/usr/bin/env bash\nexit 0\n' >"$source/payload/hook.sh"
  chmod +x "$source/payload/runtime" "$source/payload/hook.sh"
  python3 - "$source" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
lines = []
for path in sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'checksums.sha256'):
    lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(root)}")
(root / 'checksums.sha256').write_text('\n'.join(lines) + '\n')
PY
  mapfile -t package_files < <(cd "$source" && find . -type f -printf '%P\n' | sort)
  tar --sort=name --mtime='UTC 2026-08-03' --owner=0 --group=0 --numeric-owner \
    -C "$source" -cf - "${package_files[@]}" | gzip -n >"$output"
}

package_v1="$tmp/test-full-v1.tar.gz"
package_v2="$tmp/test-full-v2.tar.gz"
package_bad="$tmp/test-full-broken.tar.gz"
build_package 1.0.0 "$package_v1" 0
build_package 2.0.0 "$package_v2" 0
build_package 3.0.0 "$package_bad" 23

socket="$tmp/aletheon.sock"
log="$tmp/methods.jsonl"
python3 - "$socket" "$log" <<'PY' &
import json, os, pathlib, socket, sys
socket_path, log_path = map(pathlib.Path, sys.argv[1:])
try: socket_path.unlink()
except FileNotFoundError: pass
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(str(socket_path)); server.listen(16)
digest = 'empty'
inventory = []
while True:
    connection, _ = server.accept()
    stream = connection.makefile('rwb')
    for index in range(3):
        line = stream.readline()
        if not line: break
        request = json.loads(line)
        ident = request.get('id')
        method = request.get('method')
        if index == 0:
            result = {'protocol_version': 1}
        elif index == 1:
            result = {'status': 'ready'}
        else:
            params = request.get('params') or {}
            with log_path.open('a') as log:
                log.write(json.dumps({'method': method, 'params': params}, sort_keys=True) + '\n')
            old = digest
            operation = method.split('.', 1)[1]
            package = str(params.get('path', ''))
            if operation == 'upgrade' and 'broken' in package:
                response = {'jsonrpc':'2.0','id':ident,'error':{
                    'code':-32060,'message':'probing extension runtime candidate failed'}}
                stream.write((json.dumps(response) + '\n').encode()); stream.flush(); continue
            if operation in ('enable', 'rollback'):
                digest = 'full-v1'; inventory = ['skill','hook','agent_profile','connector','executable']
            elif operation == 'disable':
                digest = 'empty'; inventory = []
            elif operation == 'upgrade':
                digest = 'full-v2'; inventory = ['skill','hook','agent_profile','connector','executable']
            if operation == 'doctor':
                result = {'id':'test.full','healthy':True,'issues':[],
                          'snapshot_digest':digest,'inventory':inventory}
            else:
                result = {'schema_version':1,'operation':operation,'actor':'local:1000',
                          'package_id':'test.full','package_version':'1.0.0',
                          'package_hash':'a'*64,'previous_snapshot_digest':old,
                          'snapshot_digest':digest,'permission_approved':
                          bool(params.get('approve_permissions', False)),
                          'health':'healthy','evidence_references':[],
                          'inventory':inventory}
        response = {'jsonrpc':'2.0','id':ident,'result':result}
        stream.write((json.dumps(response) + '\n').encode()); stream.flush()
    stream.close(); connection.close()
PY
server_pid=$!
for _ in $(seq 1 100); do [[ -S "$socket" ]] && break; sleep 0.02; done
[[ -S "$socket" ]]

run_cli() {
  "$binary" --socket "$socket" extension "$@"
}

export ALETHEON_EXTENSION_STORE_ROOT="$tmp/store"
run_cli validate "$package_v1" >"$tmp/validate.txt"
run_cli install "$package_v1" >"$tmp/install.json"
run_cli enable test.full --approve-permissions >"$tmp/enable.json"
run_cli doctor test.full >"$tmp/doctor-v1.json"
run_cli disable test.full >"$tmp/disable.json"
run_cli enable test.full --approve-permissions >"$tmp/reenable.json"
run_cli upgrade "$package_v2" --approve-permissions >"$tmp/upgrade.json"
run_cli rollback test.full >"$tmp/rollback.json"
if run_cli upgrade "$package_bad" --approve-permissions >"$tmp/bad.json" 2>"$tmp/bad.err"; then
  echo 'failing runtime upgrade unexpectedly succeeded' >&2
  exit 1
fi
run_cli doctor test.full >"$tmp/doctor-final.json"

python3 - "$tmp" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
expected = ['skill','hook','agent_profile','connector','executable']
for name in ('enable.json','reenable.json','upgrade.json','rollback.json'):
    value = json.loads((root / name).read_text())
    if value['inventory'] != expected:
        raise SystemExit(f'{name}: assets were not published together')
if json.loads((root/'disable.json').read_text())['inventory']:
    raise SystemExit('disable retained package inventory')
if json.loads((root/'doctor-final.json').read_text())['snapshot_digest'] != 'full-v1':
    raise SystemExit('failed upgrade changed previous-known-good digest')
methods = [json.loads(line)['method'] for line in (root/'methods.jsonl').read_text().splitlines()]
expected_methods = ['extension.install','extension.enable','extension.doctor','extension.disable',
                    'extension.enable','extension.upgrade','extension.rollback',
                    'extension.upgrade','extension.doctor']
if methods != expected_methods:
    raise SystemExit(f'unexpected extension RPC sequence: {methods}')
PY

grep -q 'Package is valid' "$tmp/validate.txt"
grep -q 'probing extension runtime candidate failed' "$tmp/bad.err"
echo 'extension runtime operations acceptance passed'
