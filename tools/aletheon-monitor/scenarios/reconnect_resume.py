"""Real TUI long-output and reconnect/resume scenario."""
from __future__ import annotations
import shutil
import uuid
from pathlib import Path
from src import scenarios as base
from src.tools import tui


async def run(source_root: str, timeout: float = 180.0) -> dict:
    root = Path(source_root).resolve()
    first = await base._tui_task(root, "列出 workspace 全部 crate，并输出每个 crate 一行说明，确保产生长输出。", timeout)
    first_events = base._events(first.get("event_path"))
    receipt_root = root / ".scenario-runs" / uuid.uuid4().hex
    receipt_root.mkdir(parents=True)
    durable_event_path = receipt_root / "initial-events.jsonl"
    if first.get("event_path"):
        shutil.copyfile(first["event_path"], durable_event_path)
    durable_turns = sum(event.get("type") == "turn_done" for event in first_events)
    restarted = await tui.tui_start(working_dir=str(root), cols=110, rows=35)
    try:
        capture = await tui.tui_capture(scrollback=True, wait_stable=True, require_change=False, timeout=15)
    finally:
        await tui.tui_stop()
    frame = capture.get("frame", "")
    assertions = [
        {"name": "initial_turn_done", "passed": first.get("turn_done") is True},
        {"name": "long_output", "passed": len(first.get("frame", "")) > 1000},
        {"name": "durable_turn_event", "passed": durable_turns >= 1},
        {"name": "reconnected", "passed": restarted.get("ok") is True},
        {"name": "resume_state_rendered", "passed": bool(frame) and "aletheon" in frame.lower()},
    ]
    return {"scenario": "reconnect_resume", "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
            "assertions": assertions, "evidence": {"event_path": str(durable_event_path),
            "turn_done_events": durable_turns, "initial_frame_bytes": len(first.get("frame", "").encode()),
            "reconnect_frame_bytes": len(frame.encode())}}
