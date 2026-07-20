import asyncio
import hashlib
import os
import shlex
from pathlib import Path

import pytest

from src.tools import tui


def _install_ready_tui(monkeypatch):
    async def start(command, **kwargs):
        Path(shlex.split(command)[-1]).write_bytes(b"")
        return {"ok": True, "session": "test", "working_dir": kwargs.get("working_dir")}

    async def ready(*_args, **_kwargs):
        return "Aletheon ready"

    monkeypatch.setattr(tui.ts, "start", start)
    monkeypatch.setattr(tui, "_wait_ready", ready)
    monkeypatch.setattr(tui, "_ACTIVE_EVENT_PATH", None)


def test_tui_start_generates_unique_private_workspace_evidence(monkeypatch, tmp_path):
    _install_ready_tui(monkeypatch)

    first = asyncio.run(tui.tui_start(working_dir=str(tmp_path)))
    second = asyncio.run(tui.tui_start(working_dir=str(tmp_path)))

    first_path = first["event_path"]
    second_path = second["event_path"]
    assert first["ok"] is True and second["ok"] is True
    assert first_path != second_path
    assert os.path.commonpath((first_path, str(tmp_path))) == str(tmp_path)
    assert os.stat(first_path).st_mode & 0o777 == 0o600
    assert os.stat(second_path).st_mode & 0o777 == 0o600


def test_tui_start_uses_explicit_receipt_path_without_overwrite(monkeypatch, tmp_path):
    _install_ready_tui(monkeypatch)
    receipt = tmp_path / "receipt"
    receipt.mkdir(mode=0o700)
    event_path = receipt / "events.jsonl"

    result = asyncio.run(tui.tui_start(
        working_dir=str(tmp_path), event_path=str(event_path)
    ))
    repeated = asyncio.run(tui.tui_start(
        working_dir=str(tmp_path), event_path=str(event_path)
    ))

    assert result["ok"] is True
    assert result["event_path"] == str(event_path)
    assert repeated["ok"] is False
    assert "event setup failed" in repeated["error"]


def test_tui_start_fails_when_readiness_marker_is_missing(monkeypatch, tmp_path):
    killed = []

    async def start(command, **kwargs):
        Path(shlex.split(command)[-1]).write_bytes(b"")
        return {"ok": True, "session": "test", "working_dir": kwargs.get("working_dir")}

    async def not_ready(*_args, **_kwargs):
        return "still starting"

    async def kill(*_args, **_kwargs):
        killed.append(True)
        return {"ok": True}

    monkeypatch.setattr(tui.ts, "start", start)
    monkeypatch.setattr(tui.ts, "kill", kill)
    monkeypatch.setattr(tui, "_wait_ready", not_ready)

    result = asyncio.run(tui.tui_start(working_dir=str(tmp_path)))

    assert result["ok"] is False
    assert "readiness marker" in result["error"]
    assert killed == [True]


def test_tui_start_fails_when_event_recorder_does_not_open_file(monkeypatch, tmp_path):
    async def start(_command, **kwargs):
        return {"ok": True, "session": "test", "working_dir": kwargs.get("working_dir")}

    async def ready(*_args, **_kwargs):
        return "Aletheon ready"

    async def kill(*_args, **_kwargs):
        return {"ok": True}

    monkeypatch.setattr(tui.ts, "start", start)
    monkeypatch.setattr(tui.ts, "kill", kill)
    monkeypatch.setattr(tui, "_wait_ready", ready)

    result = asyncio.run(tui.tui_start(working_dir=str(tmp_path)))

    assert result["ok"] is False
    assert "event recorder failed" in result["error"]


def test_event_evidence_requires_private_nonempty_structured_regular_file(tmp_path):
    path = tmp_path / "events.jsonl"
    content = b'{"type":"turn_started"}\n{"type":"turn_done"}\n'
    path.write_bytes(content)
    path.chmod(0o600)

    evidence = tui.event_file_evidence(path)

    assert evidence["event_count"] == 2
    assert evidence["sha256"] == hashlib.sha256(content).hexdigest()
    assert evidence["size_bytes"] == len(content)
    assert evidence["uid"] == os.geteuid()
    assert evidence["mode"] == "0600"
    assert tui.event_evidence_matches(evidence) is True

    path.write_bytes(content + b'{"type":"late"}\n')
    assert tui.event_evidence_matches(evidence) is False
    path.write_bytes(content)

    empty = tmp_path / "empty.jsonl"
    empty.touch(mode=0o600)
    with pytest.raises(RuntimeError, match="empty"):
        tui.event_file_evidence(empty)

    unsafe = tmp_path / "unsafe.jsonl"
    unsafe.write_bytes(content)
    unsafe.chmod(0o644)
    with pytest.raises(RuntimeError, match="ownership or mode"):
        tui.event_file_evidence(unsafe)

    link = tmp_path / "link.jsonl"
    link.symlink_to(path)
    with pytest.raises(RuntimeError, match="regular file"):
        tui.event_file_evidence(link)


def test_wait_turn_done_returns_validated_event_receipt(monkeypatch, tmp_path):
    _install_ready_tui(monkeypatch)
    receipt = tmp_path / "receipt"
    receipt.mkdir(mode=0o700)
    path = receipt / "events.jsonl"
    started = asyncio.run(tui.tui_start(
        working_dir=str(tmp_path), event_path=str(path)
    ))
    path.write_text('{"type":"turn_done"}\n', encoding="utf-8")

    async def capture(**_kwargs):
        return {"stable": True, "frame": "done", "checks": []}

    monkeypatch.setattr(tui, "tui_capture", capture)
    result = asyncio.run(tui.tui_wait_turn_done(started["turn_done_count"], timeout=1))

    assert result["turn_done"] is True
    assert result["event_evidence"]["event_count"] == 1
    assert result["event_evidence"]["path"] == str(path.resolve())
