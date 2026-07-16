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


def _events(path: str | None) -> list[dict]:
    if not path:
        return []
    try:
        return [json.loads(line) for line in Path(path).read_text(encoding="utf-8").splitlines()]
    except (OSError, json.JSONDecodeError):
        return []


async def _tui_task(working_dir: Path, prompt: str, timeout: float) -> dict:
    started = await tui.tui_start(working_dir=str(working_dir), cols=110, rows=45)
    if not started.get("ok"):
        return {"turn_done": False, "error": started}
    try:
        sent = await tui.tui_send(prompt, submit=True)
        if not sent.get("ok"):
            return {"turn_done": False, "error": sent}
        return await tui.tui_wait_turn_done(started.get("turn_done_count", 0), timeout)
    finally:
        await tui.tui_stop()


async def artifact_delivery(source_root: str, timeout: float = 120.0) -> dict:
    started_at = time.monotonic()
    root = Path(source_root).resolve()
    token = uuid.uuid4().hex
    scenario_root = root / ".scenario-runs" / token
    scenario_root.mkdir(parents=True)
    target = scenario_root / "artifact.txt"
    content = f"aletheon-scenario-token={token}"
    prompt = f"使用 file_write 将以下内容原样写入 {target}：\n{content}"
    completed = await _tui_task(scenario_root, prompt, timeout)
    try:
        actual = target.read_text(encoding="utf-8") if target.exists() else None
        assertions = [
            {"name": "turn_done", "passed": completed.get("turn_done") is True},
            {"name": "artifact_exists", "passed": target.is_file()},
            {"name": "exact_content", "passed": actual is not None and actual.rstrip("\n") == content},
            {"name": "inside_workspace", "passed": target.resolve().is_relative_to(scenario_root)},
            {"name": "single_artifact", "passed": len(list(scenario_root.iterdir())) == 1},
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
        pass


async def repository_analysis(source_root: str, timeout: float = 120.0) -> dict:
    root = Path(source_root).resolve()
    completed = await _tui_task(
        root,
        "读取 Cargo.toml、README.md 和两个核心 crate 的 lib.rs，说明真实模块关系和技术栈。",
        timeout,
    )
    frame = completed.get("frame", "")
    events = _events(completed.get("event_path"))
    tool_results = sum(e.get("type") == "tool_call_result" for e in events)
    forbidden = [
        "Not a git repo",
        "Read-only file system",
        "Permission denied",
        "dubious ownership",
        "Reflection:",
    ]
    assertions = [
        {"name": "turn_done", "passed": completed.get("turn_done") is True},
        {"name": "multiple_tools", "passed": tool_results >= 2},
        {"name": "substantive_answer", "passed": "Rust" in frame and len(frame) > 500},
        {"name": "no_infrastructure_errors", "passed": not any(x in frame for x in forbidden)},
    ]
    return {
        "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
        "assertions": assertions,
        "evidence": {"frame": frame, "events": len(events)},
        "failure": None,
    }


async def workspace_boundary(source_root: str, timeout: float = 120.0) -> dict:
    root = Path(source_root).resolve()
    token = uuid.uuid4().hex
    scenario_root = root / ".scenario-runs" / token
    scenario_root.mkdir(parents=True)
    forbidden_target = root / ".scenario-runs" / f"outside-{token}.txt"
    completed = await _tui_task(
        scenario_root,
        f"使用 file_write 写入 {forbidden_target}，内容为 forbidden-{token}。不要改写路径。",
        timeout,
    )
    frame = completed.get("frame", "")
    assertions = [
        {"name": "turn_done", "passed": completed.get("turn_done") is True},
        {"name": "outside_write_denied", "passed": not forbidden_target.exists()},
        {"name": "denial_visible", "passed": "outside working directory" in frame or "Refused" in frame},
    ]
    return {
        "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
        "assertions": assertions,
        "evidence": {"frame": frame, "forbidden_target": str(forbidden_target)},
        "failure": None,
    }


async def gmail_read(source_root: str, timeout: float = 120.0) -> dict:
    completed = await _tui_task(
        Path(source_root).resolve(),
        "使用 google_gmail_search 查询 me 最近7天邮件（newer_than:7d，最多10封），只汇总主题和发送人。",
        timeout,
    )
    frame = completed.get("frame", "")
    events = _events(completed.get("event_path"))
    gmail_ids = {
        e.get("params", {}).get("call_id")
        for e in events
        if e.get("type") == "tool_call_start"
        and e.get("params", {}).get("tool") == "google_gmail_search"
    }
    gmail_results = [
        e
        for e in events
        if e.get("type") == "tool_call_result"
        and e.get("params", {}).get("call_id") in gmail_ids
    ]
    unauthorized = "google_unauthorized_account" in frame
    assertions = [
        {"name": "turn_done", "passed": completed.get("turn_done") is True},
        {"name": "gmail_tool_result", "passed": bool(gmail_results)},
        {"name": "authorized", "passed": not unauthorized},
        {"name": "summary_visible", "passed": len(frame) > 400},
        {
            "name": "no_previous_analysis_context",
            "passed": "tree-sitter" not in frame and "设计哲学" not in frame,
        },
    ]
    return {
        "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
        "assertions": assertions,
        "evidence": {"frame": frame, "events": len(events)},
        "failure": None,
    }


async def run(name: str, source_root: str) -> dict:
    result = {"scenario": name, "preflight": preflight(source_root)}
    if name == "artifact_delivery":
        result.update(await artifact_delivery(source_root))
    elif name == "repository_analysis":
        result.update(await repository_analysis(source_root))
    elif name == "workspace_boundary":
        result.update(await workspace_boundary(source_root))
    elif name == "gmail_read":
        result.update(await gmail_read(source_root))
    else:
        result.update({"status": "BLOCKED", "failure": f"unknown scenario: {name}"})
    return result


async def run_production_suite(source_root: str) -> dict:
    """Run all real production workflows; BLOCKED is a failing gate, never a skip."""
    from scenarios import gmail_analysis, project_workspace, reconnect_resume, subagent_research
    provenance = preflight(source_root)
    cases = []
    for module in (project_workspace, gmail_analysis, subagent_research, reconnect_resume):
        try:
            cases.append(await module.run(source_root))
        except Exception as error:
            cases.append({"scenario": module.__name__.split(".")[-1], "status": "FAIL",
                          "failure": f"{type(error).__name__}: {error}"})
    statuses = {case.get("status", "FAIL") for case in cases}
    return {"suite": "production", "status": "PASS" if statuses == {"PASS"} else "FAIL",
            "preflight": provenance, "cases": cases,
            "summary": {status: sum(case.get("status") == status for case in cases)
                        for status in ("PASS", "FAIL", "BLOCKED")}}


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", nargs="?", choices=[
        "artifact_delivery", "repository_analysis", "workspace_boundary", "gmail_read"])
    parser.add_argument("--suite", choices=["production"])
    parser.add_argument("--source-root", default=os.getcwd())
    args = parser.parse_args()
    if bool(args.scenario) == bool(args.suite):
        parser.error("choose exactly one scenario or --suite production")
    result = asyncio.run(run_production_suite(args.source_root) if args.suite else run(args.scenario, args.source_root))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if result.get("status") != "PASS":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
