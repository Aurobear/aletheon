#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
validator="$repo_root/scripts/libexec/aletheon/extension-acceptance.sh"
bash -n "$validator"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
python3 - "$tmp" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
names = (
    "candidate-hash.json", "installed-hash.json", "running-process-hash.json",
    "daemon-health.json", "cli-install.json", "tui-tool-task.json",
    "subagent-runtime-task.json", "profile-quarantine.json",
    "connector-failure.json", "runtime-crash.json", "upgrade-rollback.json",
    "cleanup.json",
)
digest = "a" * 64
for index, name in enumerate(names):
    value = {
        "schema_version": 1,
        "commit": "abcdef1",
        "command": ["fixture", name],
        "started_at": "2026-07-24T00:00:00Z",
        "ended_at": "2026-07-24T00:00:01Z",
        "status": "PASS",
        "evidence_summary": {"fixture": True},
        "acceptance_criteria": [index + 1],
    }
    if name in {"candidate-hash.json", "installed-hash.json", "running-process-hash.json"}:
        value["sha256"] = digest
    if name == "cleanup.json":
        value["evidence_summary"] = {"remaining_processes": 0, "active_pointers": 0}
    (root / name).write_text(json.dumps(value) + "\n")
events = []
for criterion in range(13, 21):
    events.append(json.dumps({
        "schema_version": 1,
        "commit": "abcdef1",
        "command": ["fixture", "event"],
        "started_at": "2026-07-24T00:00:00Z",
        "ended_at": "2026-07-24T00:00:01Z",
        "status": "PASS",
        "evidence_summary": {"event": criterion},
        "acceptance_criteria": [criterion],
    }))
(root / "metacog-events.jsonl").write_text("\n".join(events) + "\n")
PY

bash "$validator" "$tmp"
python3 - "$tmp/cleanup.json" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text())
value["evidence_summary"]["remaining_processes"] = 1
path.write_text(json.dumps(value) + "\n")
PY
if bash "$validator" "$tmp" >/dev/null 2>&1; then
  echo "validator accepted a dirty cleanup receipt" >&2
  exit 1
fi
echo "extension acceptance static verification: pass"
