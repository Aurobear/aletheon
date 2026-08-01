import asyncio
from pathlib import Path

from src import scenarios


def test_preflight_records_real_provenance(monkeypatch, tmp_path):
    binary = tmp_path / "aletheon"
    binary.write_bytes(b"binary")
    values = iter([
        str(binary), "active", "active", "deadbeef",
        "Id=aletheon.service\nActiveState=active\nId=aletheon.socket\nActiveState=active",
    ])
    monkeypatch.setattr(scenarios, "_command", lambda *a, **k: next(values))
    monkeypatch.setattr(
        scenarios, "_socket_provenance",
        lambda: {"path": "/run/user/1000/aletheon/aletheon.sock", "uid": 1000,
                 "gid": 1000, "mode": "0600"},
    )
    result = scenarios.preflight(str(tmp_path))
    assert result["source_commit"] == "deadbeef"
    assert result["binary_sha256"]
    assert "ActiveState=active" in result["unit"]


def test_workspace_boundary_keeps_event_evidence_outside_tested_root(monkeypatch, tmp_path):
    observed = {}

    async def fake_tui_task(working_dir, prompt, timeout, event_path):
        target = working_dir.parent / f"outside-{working_dir.name}.txt"
        denial = f"Refused to write {target}: outside admitted workspace roots"
        observed.update(
            working_dir=working_dir,
            prompt=prompt,
            timeout=timeout,
            event_path=event_path,
            target=target,
            content=f"forbidden-{working_dir.name}",
            denial=denial,
        )
        return {
            "turn_done": True,
            "frame": denial,
            "event_path": str(event_path),
            "event_evidence": {"path": str(event_path)},
        }

    monkeypatch.setattr(scenarios, "_tui_task", fake_tui_task)
    monkeypatch.setattr(scenarios.tui, "event_evidence_matches", lambda _value: True)
    monkeypatch.setattr(
        scenarios,
        "_events",
        lambda _path: [
            {
                "type": "tool_call_complete",
                "params": {
                    "tool": "file_write",
                    "call_id": "write-1",
                    "args": {
                        "path": str(observed["target"]),
                        "content": observed["content"],
                        "transaction_id": "tx-1",
                    },
                },
            },
            {
                "type": "tool_call_result",
                "params": {
                    "tool": "file_write",
                    "call_id": "write-1",
                    "is_error": True,
                    "output": observed["denial"],
                },
            },
        ],
    )

    result = asyncio.run(scenarios.workspace_boundary(str(tmp_path), timeout=17))

    assert result["status"] == "PASS"
    assert {item["name"] for item in result["assertions"] if item["passed"]} >= {
        "runtime_denial_receipt",
        "denial_visible",
    }
    assert observed["timeout"] == 17
    assert observed["event_path"].name == "events.jsonl"
    assert observed["event_path"].parent != observed["working_dir"]
    assert observed["event_path"].parent.parent == observed["working_dir"].parent
