#!/usr/bin/env bash
set -euo pipefail

umask 077

ALETHEON_BIN=${ALETHEON_BIN:-/usr/bin/aletheon}
PI_BIN=${PI_BIN:-/usr/bin/pi}
SOCKET=${ALETHEON_SOCKET:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock}
WORKSPACE=${ALETHEON_PI_WORKSPACE:-$HOME/.local/state/aletheon/scheduled-fixture}
EVIDENCE_DIR=${ALETHEON_PI_EVIDENCE_DIR:-$HOME/.local/state/aletheon/scheduled-evidence}
LOCK_FILE=${ALETHEON_PI_LOCK_FILE:-$HOME/.local/state/aletheon/scheduled-task.lock}
TIMEOUT_SECONDS=${ALETHEON_PI_TIMEOUT_SECONDS:-300}

mkdir -p -- "$EVIDENCE_DIR" "$(dirname -- "$LOCK_FILE")"
chmod 0700 -- "$EVIDENCE_DIR"
if [[ -L "$EVIDENCE_DIR" || ! -d "$EVIDENCE_DIR" ]]; then
    echo "unsafe scheduled evidence directory" >&2
    exit 70
fi
if [[ ! -S "$SOCKET" ]]; then
    echo "Aletheon user socket is unavailable" >&2
    exit 69
fi
if [[ ! -x "$ALETHEON_BIN" || ! -x "$PI_BIN" ]]; then
    echo "reviewed Aletheon or Pi executable is unavailable" >&2
    exit 69
fi
if ! git -C "$WORKSPACE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "scheduled fixture is not a Git worktree" >&2
    exit 66
fi

exec 9>"$LOCK_FILE"
if ! flock --nonblock 9; then
    echo "scheduled Pi task already running" >&2
    exit 75
fi

stamp=$(date -u +%Y%m%dT%H%M%SZ)
receipt="$EVIDENCE_DIR/$stamp.jsonl"
prompt_file=$(mktemp "$EVIDENCE_DIR/.prompt.XXXXXX")
trap 'rm -f -- "$prompt_file"' EXIT

python3 - "$WORKSPACE" "$PI_BIN" >"$prompt_file" <<'PY'
import json
import pathlib
import subprocess
import sys
import uuid

workspace = str(pathlib.Path(sys.argv[1]).resolve(strict=True))
pi_bin = str(pathlib.Path(sys.argv[2]).resolve(strict=True))
base = subprocess.check_output(
    ["git", "-C", workspace, "rev-parse", "HEAD"], text=True
).strip()
request = {
    "job": {
        "job_id": str(uuid.uuid4()),
        "goal_id": 7100,
        "attempt_id": str(uuid.uuid4()),
        "workspace": {
            "repository_root": workspace,
            "allowed_paths": ["src/lib.rs"],
            "forbidden_paths": [".git", ".env", ".aletheon"],
        },
        "base_commit": base,
        "command": pi_bin,
        "args": [
            "--mode", "json", "--no-session", "--no-context-files",
            "--no-extensions", "--no-skills", "--no-prompt-templates",
            "--no-themes", "--no-approve", "--offline",
            "--provider", "leju", "--model", "deepseek/deepseek-v4-pro",
        ],
        "timeout_ms": 180000,
        "output_cap_bytes": 8388608,
        "network_policy": {"mode": "disabled"},
    },
    "task_input": (
        "Inspect src/lib.rs and ensure status() returns the literal updated. "
        "Change only src/lib.rs if needed. Do not run tests, create other files, "
        "or commit. Give a concise final summary."
    ),
}
print(
    "必须实际调用 agent_spawn，profile=code-agent，runtime=pi-coder，task 使用 "
    "TASK_JSON 单行字符串，budget 为 max_input_tokens=8000,"
    "max_output_tokens=3000,max_tool_calls=12,max_elapsed_ms=240000,"
    "max_depth=1；随后 agent_wait 240000ms，并报告终态。\nTASK_JSON="
    + json.dumps(request, separators=(",", ":"))
)
PY

set +e
timeout --signal=TERM --kill-after=15 "$TIMEOUT_SECONDS" \
    "$ALETHEON_BIN" --socket "$SOCKET" -C "$WORKSPACE" \
    -m "$(cat -- "$prompt_file")" >"$receipt" 2>&1
status=$?
set -e
chmod 0600 -- "$receipt"
printf '{"completed_at":"%s","exit_status":%d}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$status" >>"$receipt"
exit "$status"
