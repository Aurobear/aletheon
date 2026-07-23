#!/usr/bin/env bash
set -euo pipefail

receipt_dir=${1:-}
if [[ -z "$receipt_dir" ]]; then
  echo "usage: extension-acceptance.sh RECEIPT_DIRECTORY" >&2
  exit 2
fi
[[ -d "$receipt_dir" && ! -L "$receipt_dir" ]] || {
  echo "BLOCKED: extension receipt directory is missing or unsafe: $receipt_dir" >&2
  exit 78
}

python3 - "$receipt_dir" <<'PY'
import json, pathlib, re, sys

root = pathlib.Path(sys.argv[1])
required = (
    "candidate-hash.json",
    "installed-hash.json",
    "running-process-hash.json",
    "daemon-health.json",
    "cli-install.json",
    "tui-tool-task.json",
    "subagent-runtime-task.json",
    "profile-quarantine.json",
    "connector-failure.json",
    "runtime-crash.json",
    "upgrade-rollback.json",
    "metacog-events.jsonl",
    "cleanup.json",
)
sha256 = re.compile(r"[0-9a-f]{64}").fullmatch
commit = re.compile(r"[0-9a-f]{7,64}").fullmatch
secret_key = re.compile(
    r"(^|[_-])(api[_-]?key|authorization|password|secret|token)($|[_-])", re.I
)

def fail(message):
    raise SystemExit(f"extension acceptance: {message}")

def safe_file(name):
    path = root / name
    if not path.is_file() or path.is_symlink():
        fail(f"required receipt is missing or unsafe: {name}")
    return path

def reject_secrets(value, location="receipt"):
    if isinstance(value, dict):
        for key, child in value.items():
            if secret_key.search(str(key)) and child not in (None, "", "<redacted>", "[redacted]"):
                fail(f"unredacted secret-bearing field at {location}.{key}")
            reject_secrets(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_secrets(child, f"{location}[{index}]")

def validate_envelope(value, name):
    if not isinstance(value, dict):
        fail(f"{name} is not an object")
    if value.get("schema_version") != 1:
        fail(f"{name} schema_version is not 1")
    if not isinstance(value.get("commit"), str) or not commit(value["commit"]):
        fail(f"{name} has invalid commit")
    if not isinstance(value.get("command"), (str, list)) or not value["command"]:
        fail(f"{name} has no command")
    for field in ("started_at", "ended_at"):
        if not isinstance(value.get(field), str) or "T" not in value[field]:
            fail(f"{name} has invalid {field}")
    if value.get("status") != "PASS":
        fail(f"{name} did not pass")
    if value.get("evidence_summary") in (None, "", {}, []):
        fail(f"{name} has no evidence summary")
    criteria = value.get("acceptance_criteria")
    if not isinstance(criteria, list) or any(
        not isinstance(item, int) or isinstance(item, bool) or not 1 <= item <= 20
        for item in criteria
    ):
        fail(f"{name} has invalid acceptance criteria")
    reject_secrets(value, name)
    return value

receipts = {}
criteria = set()
for name in required:
    path = safe_file(name)
    if name.endswith(".jsonl"):
        lines = [line for line in path.read_text().splitlines() if line.strip()]
        if not lines:
            fail(f"{name} is empty")
        values = []
        for index, line in enumerate(lines, 1):
            try:
                value = json.loads(line)
            except json.JSONDecodeError as error:
                fail(f"{name}:{index} is invalid JSON: {error}")
            values.append(validate_envelope(value, f"{name}:{index}"))
        receipts[name] = values
        for value in values:
            criteria.update(value["acceptance_criteria"])
    else:
        try:
            value = json.loads(path.read_text())
        except json.JSONDecodeError as error:
            fail(f"{name} is invalid JSON: {error}")
        receipts[name] = validate_envelope(value, name)
        criteria.update(value["acceptance_criteria"])

if criteria != set(range(1, 21)):
    fail(f"acceptance criteria coverage is incomplete: {sorted(criteria)}")

hashes = []
for name in ("candidate-hash.json", "installed-hash.json", "running-process-hash.json"):
    value = receipts[name].get("sha256")
    if not isinstance(value, str) or not sha256(value):
        fail(f"{name} has invalid sha256")
    hashes.append(value)
if len(set(hashes)) != 1:
    fail("candidate, installed, and running process hashes differ")

cleanup = receipts["cleanup.json"]["evidence_summary"]
if not isinstance(cleanup, dict):
    fail("cleanup evidence summary is not an object")
if cleanup.get("remaining_processes") != 0 or cleanup.get("active_pointers") != 0:
    fail("cleanup left extension processes or active pointers")

print("extension platform acceptance receipts verified")
PY
