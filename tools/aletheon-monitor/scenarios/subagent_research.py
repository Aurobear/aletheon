"""Real SubAgent lifecycle scenario validated from recorded daemon events."""
from __future__ import annotations
from pathlib import Path
from src import scenarios as base


async def run(source_root: str, timeout: float = 180.0) -> dict:
    root = Path(source_root).resolve()
    prompt = ("启动一个有界 SubAgent 研究 Cargo workspace 的 crate 数量；展示进度和 mailbox，"
              "取消一个第二个测试 SubAgent，并将第一个结果提升为最终结论。")
    completed = await base._tui_task(root, prompt, timeout)
    events = base._events(completed.get("event_path"))
    encoded = "\n".join(str(event) for event in events).lower()
    lifecycle = {name: name in encoded for name in ("spawn", "progress", "mailbox", "cancel", "promot")}
    call_ids = {event.get("params", {}).get("call_id") for event in events if event.get("type") == "tool_call_start"}
    results = {event.get("params", {}).get("call_id") for event in events if event.get("type") == "tool_call_result"}
    assertions = [
        {"name": "authoritative_turn_done", "passed": completed.get("turn_done") is True},
        {"name": "lifecycle_evidence", "passed": all(lifecycle.values())},
        {"name": "tool_results_accounted", "passed": bool(call_ids) and call_ids <= results},
        {"name": "durable_event_log", "passed": bool(events)},
    ]
    return {"scenario": "subagent_research", "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
            "assertions": assertions, "evidence": {"event_path": completed.get("event_path"),
            "event_count": len(events), "lifecycle": lifecycle, "tool_calls": len(call_ids)}}
