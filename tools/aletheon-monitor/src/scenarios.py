"""Production-path Aletheon scenario runner.

Scenarios drive the installed TUI and verify effects from the host. They do
not treat an RPC response, output length, or a visible prompt as completion.
"""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
import subprocess
import time
import uuid
from pathlib import Path

from .tools import tui


def _command(*args: str, cwd: str | None = None) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True, stderr=subprocess.STDOUT).strip()


def preflight(source_root: str) -> dict:
    root = os.path.realpath(source_root)
    binary = _command("bash", "-lc", "command -v aletheon")
    digest = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
    return {
        "source_root": root,
        "source_commit": _command("git", "rev-parse", "HEAD", cwd=root),
        "binary": binary,
        "binary_sha256": digest,
        "unit": _command(
            "systemctl", "show", "aletheon", "-p", "ActiveState",
            "-p", "ActiveEnterTimestamp", "-p", "ExecStart", "-p", "ReadWritePaths"
        ),
    }


async def artifact_delivery(source_root: str, timeout: float = 120.0) -> dict:
    started_at = time.monotonic()
    root = Path(source_root).resolve()
    token = uuid.uuid4().hex
    relative = Path("docs/plans") / f"aletheon-scenario-{token}.md"
    target = root / relative
    content = f"# Aletheon scenario artifact\n\nverification-token: {token}\n"
    prompt = f"使用 file_write 将以下内容原样写入 {target}：\n{content}"
    started = await tui.tui_start(working_dir=str(root), cols=110, rows=45)
    if not started.get("ok"):
        return {"status": "FAIL", "failure": started, "assertions": []}
    try:
        await tui.tui_send(prompt, submit=True)
        completed = await tui.tui_wait_turn_done(started.get("turn_done_count", 0), timeout)
        actual = target.read_text(encoding="utf-8") if target.exists() else None
        assertions = [
            {"name": "turn_done", "passed": completed.get("turn_done") is True},
            {"name": "artifact_exists", "passed": target.is_file()},
            {"name": "exact_content", "passed": actual == content},
            {"name": "inside_workspace", "passed": target.resolve().is_relative_to(root)},
        ]
        return {
            "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
            "assertions": assertions,
            "duration_ms": round((time.monotonic() - started_at) * 1000),
            "evidence": {"target": str(target), "frame": completed.get("frame", ""),
                         "event_path": completed.get("event_path")},
            "failure": None,
        }
    finally:
        await tui.tui_stop()


async def run(name: str, source_root: str) -> dict:
    result = {"scenario": name, "preflight": preflight(source_root)}
    if name == "artifact_delivery":
        result.update(await artifact_delivery(source_root))
    else:
        result.update({"status": "BLOCKED", "failure": f"unknown scenario: {name}"})
    return result


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", choices=["artifact_delivery"])
    parser.add_argument("--source-root", default=os.getcwd())
    args = parser.parse_args()
    print(json.dumps(asyncio.run(run(args.scenario, args.source_root)), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
