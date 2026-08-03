#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
binary=${ALETHEON_COMMAND_SURFACE_BIN:-/usr/bin/aletheon}
socket=${ALETHEON_USER_SOCKET:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock}

[[ -x "$binary" ]] || {
  echo "installed Aletheon binary is not executable: $binary" >&2
  exit 1
}
[[ -S "$socket" ]] || {
  echo "official Aletheon user socket is unavailable: $socket" >&2
  exit 1
}

bash "$root/scripts/cargo-agent.sh" test -p interact --lib \
  public_builtin_command_set_is_exact -- --nocapture
bash "$root/scripts/cargo-agent.sh" test -p interact --lib \
  retired_governance_commands_parse_as_unknown -- --nocapture

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

printf '/help\n' >"$tmp/help.input"
"$binary" --socket "$socket" \
  --test-input "$tmp/help.input" \
  --record-frames "$tmp/help.frames.jsonl" \
  --auto-submit --test-timeout 2

cat >"$tmp/retired.input" <<'EOF'
/reflect
/reflect_now
/evolution
/genome
/hooks
/task
/evaluation
/approve
/plan
/computer
EOF
"$binary" --socket "$socket" \
  --test-input "$tmp/retired.input" \
  --record-frames "$tmp/retired.frames.jsonl" \
  --auto-submit --test-timeout 12

python3 - "$tmp/help.frames.jsonl" "$tmp/retired.frames.jsonl" <<'PY'
import json
import pathlib
import sys


def contents(path: str) -> list[str]:
    return [
        json.loads(line)["content"]
        for line in pathlib.Path(path).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


help_frames = contents(sys.argv[1])
retired_frames = contents(sys.argv[2])
if not help_frames:
    raise SystemExit("TUI /help produced no recorded frame")

help_view = "\n".join(help_frames)
if not any(marker in help_view for marker in ("Aletheon 命令", "/memory", "/status")):
    raise SystemExit("recorded /help frames do not show the public command catalog")

retired = [
    "reflect",
    "reflect_now",
    "evolution",
    "genome",
    "hooks",
    "task",
    "evaluation",
    "approve",
    "plan",
    "computer",
]
for name in retired:
    if f"/{name}" in help_view:
        raise SystemExit(f"retired /{name} remains visible in installed TUI help")

retired_view = "".join("\n".join(retired_frames).split())
for name in retired:
    if f"未知命令/{name}" not in retired_view:
        raise SystemExit(f"installed TUI did not reject /{name} as unknown")
PY

echo "installed TUI command surface passed"
