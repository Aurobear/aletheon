import asyncio
import json
from src.tools import tui as tui_tools
from src.tools import diagnose as diag
from src.tools.diagnose import (
    build_timeline,
    event_acceptance,
    frame_assertions,
    provider_metrics,
)


def test_diagnose_stops_tui_on_capture_error(monkeypatch):
    calls = {"stopped": False}
    async def ok_start(task="", cols=120, rows=50, working_dir=None):
        return {"ok": True, "session": "s", "frame": ""}
    async def ok_send(text, submit=True):
        return {"ok": True}
    async def boom_capture(*args, **kwargs):
        raise RuntimeError("capture failed")
    async def rec_stop():
        calls["stopped"] = True
        return {"ok": True}
    monkeypatch.setattr(tui_tools, "tui_start", ok_start)
    monkeypatch.setattr(tui_tools, "tui_send", ok_send)
    monkeypatch.setattr(tui_tools, "tui_capture", boom_capture)
    monkeypatch.setattr(tui_tools, "tui_stop", rec_stop)
    try:
        asyncio.run(diag.diagnose(client=None, task="x"))
    except RuntimeError:
        pass
    assert calls["stopped"] is True


def test_diagnose_creates_fresh_session_before_task(monkeypatch):
    sent = []

    async def ok_start(task="", cols=120, rows=50, working_dir=None):
        return {"ok": True, "session": "s", "frame": "", "turn_done_count": 0}

    async def record_send(text, submit=True):
        sent.append(text)
        return {"ok": True}

    async def capture(*args, **kwargs):
        return {"frame": "ready", "stable": True}

    async def done(*args, **kwargs):
        return {
            "frame": "answer\n❯",
            "stable": True,
            "turn_done": True,
            "turn_done_count": 1,
            "completion_source": "client_event:turn_done",
            "event_path": "/tmp/events.jsonl",
            "event_evidence": {"sha256": "abc"},
            "prompt_visible": True,
            "checks": [],
        }

    async def stop():
        return {"ok": True}

    async def analyze(_client):
        return {"healthy": True, "recent_journal": []}

    async def logs(_client, last_n=50):
        return {"lines": []}

    monkeypatch.setattr(tui_tools, "tui_start", ok_start)
    monkeypatch.setattr(tui_tools, "tui_send", record_send)
    monkeypatch.setattr(tui_tools, "tui_capture", capture)
    monkeypatch.setattr(tui_tools, "tui_wait_turn_done", done)
    monkeypatch.setattr(tui_tools, "tui_stop", stop)
    monkeypatch.setattr(diag.analyze_mod, "analyze", analyze)
    monkeypatch.setattr(diag.logs_mod, "logs", logs)
    monkeypatch.setattr(diag, "_audit_tail", lambda: [])

    result = asyncio.run(diag.diagnose(client=None, task="inspect"))

    assert sent == ["/new", "inspect"]
    assert result["completion_source"] == "client_event:turn_done"
    assert result["event_path"] == "/tmp/events.jsonl"
    assert result["event_evidence"] == {"sha256": "abc"}


def test_expected_cwd_uses_authoritative_tmux_launch_state(monkeypatch, tmp_path):
    async def ok_start(task="", cols=120, rows=50, working_dir=None):
        return {"ok": True, "session": "s", "frame": "", "turn_done_count": 0,
                "working_dir": str(tmp_path)}

    async def ok_send(text, submit=True):
        return {"ok": True}

    async def capture(*args, **kwargs):
        return {"frame": "ready", "stable": True}

    async def done(*args, **kwargs):
        return {"frame": "answer without cwd\n❯", "stable": True,
                "turn_done": True, "prompt_visible": True, "checks": []}

    async def noop(*args, **kwargs):
        return {"healthy": True, "recent_journal": [], "lines": []}

    monkeypatch.setattr(tui_tools, "tui_start", ok_start)
    monkeypatch.setattr(tui_tools, "tui_send", ok_send)
    monkeypatch.setattr(tui_tools, "tui_capture", capture)
    monkeypatch.setattr(tui_tools, "tui_wait_turn_done", done)
    monkeypatch.setattr(tui_tools, "tui_stop", lambda: noop())
    monkeypatch.setattr(diag.analyze_mod, "analyze", noop)
    monkeypatch.setattr(diag.logs_mod, "logs", noop)
    monkeypatch.setattr(diag, "_audit_tail", lambda: [])

    result = asyncio.run(diag.diagnose(
        client=None, task="inspect", working_dir=str(tmp_path),
        expected_cwd=str(tmp_path),
    ))

    assert next(a for a in result["assertions"] if a["name"] == "expected_cwd")["passed"]


def test_build_timeline_sorts_sources_by_timestamp():
    journal = [
        {"timestamp": "2026-07-06T11:15:01Z", "type": "user_message"},
        {"timestamp": "2026-07-06T11:15:03Z", "type": "reflection"},
    ]
    audit = [
        '{"timestamp":"2026-07-06T11:15:02Z","tool_name":"glob"}',
        'not json — ignored',
    ]
    tl = build_timeline(journal, audit)
    assert [e["source"] for e in tl] == ["journal", "audit", "journal"]
    assert tl[0]["ts"] == "2026-07-06T11:15:01Z"
    assert tl[1]["summary"].startswith("glob") or "glob" in tl[1]["summary"]


def test_provider_error_frame_cannot_pass_as_final_answer():
    assertions = frame_assertions(
        "│ hello\nerror: cognitive session: provider_unavailable\n❯",
        prompt_visible=True,
    )
    assert next(a for a in assertions if a["name"] == "final_answer")["passed"] is False
    assert next(
        a for a in assertions if a["name"] == "forbidden:provider_unavailable"
    )["passed"] is False


def test_provider_timeout_frame_cannot_pass_as_final_answer():
    assertions = frame_assertions(
        "partial answer\nerror: cognitive session TerminalRuntime: provider_timeout\n❯",
        prompt_visible=True,
    )
    assert next(a for a in assertions if a["name"] == "final_answer")["passed"] is False
    assert next(
        a for a in assertions if a["name"] == "forbidden:provider_timeout"
    )["passed"] is False


def test_provider_error_code_discussed_in_answer_is_not_rendered_failure():
    assertions = frame_assertions(
        "主要风险：provider_unavailable 仍然会使对应验收失败。\n❯",
        prompt_visible=True,
    )
    assert next(
        a for a in assertions if a["name"] == "forbidden:provider_unavailable"
    )["passed"] is True
    assert next(a for a in assertions if a["name"] == "final_answer")["passed"] is True


def test_repository_overview_requires_repo_inspect_before_scoped_discovery(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {
            "type": "tool_call_complete",
            "params": {
                "tool": "glob",
                "args": {"patterns": ["**/*.rs"]},
            },
        },
        {
            "type": "tool_call_complete",
            "params": {"tool": "repo_inspect", "args": {"root": "/repo"}},
        },
        {"type": "text_snapshot", "params": {"text": "A" * 80 + "."}},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertions = {item["name"]: item for item in accepted["assertions"]}
    assert assertions["repository_overview_starts_with_repo_inspect"]["passed"] is False
    assert assertions["repository_overview_waits_for_repo_inspect"]["passed"] is False
    assert assertions["repository_overview_avoids_broad_glob"]["passed"] is False


def test_repository_overview_rejects_recursive_docs_and_wildcard_scope_inventories(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {
            "type": "tool_call_complete",
            "params": {"call_id": "repo", "tool": "repo_inspect", "args": {}},
        },
        {
            "type": "tool_call_result",
            "params": {"call_id": "repo", "tool": "repo_inspect", "is_error": False},
        },
        {
            "type": "tool_call_complete",
            "params": {
                "call_id": "glob",
                "tool": "glob",
                "args": {"patterns": ["docs/**/*.md", "crates/*/tests/**/*.rs"]},
            },
        },
        {"type": "text_snapshot", "params": {"text": "A" * 80 + "."}},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertion = next(
        item for item in accepted["assertions"]
        if item["name"] == "repository_overview_avoids_broad_glob"
    )
    assert assertion["passed"] is False
    assert assertion["patterns"] == ["docs/**/*.md", "crates/*/tests/**/*.rs"]


def test_repository_overview_rejects_large_batched_inventory(tmp_path):
    path = tmp_path / "events.jsonl"
    patterns = [f"crates/crate-{index}/src/**/*.rs" for index in range(7)]
    records = [
        {
            "type": "tool_call_complete",
            "params": {"call_id": "repo", "tool": "repo_inspect", "args": {}},
        },
        {
            "type": "tool_call_result",
            "params": {"call_id": "repo", "tool": "repo_inspect", "is_error": False},
        },
        {
            "type": "tool_call_complete",
            "params": {"call_id": "glob", "tool": "glob", "args": {"patterns": patterns}},
        },
        {"type": "text_snapshot", "params": {"text": "A" * 80 + "."}},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertion = next(
        item for item in accepted["assertions"]
        if item["name"] == "repository_overview_avoids_broad_glob"
    )
    assert assertion["passed"] is False
    assert assertion["patterns"] == patterns


def test_repository_overview_waits_for_repo_inspect_result(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {
            "type": "tool_call_complete",
            "params": {"call_id": "repo", "tool": "repo_inspect", "args": {}},
        },
        {
            "type": "tool_call_complete",
            "params": {"call_id": "search", "tool": "file_search", "args": {}},
        },
        {
            "type": "tool_call_result",
            "params": {"call_id": "repo", "tool": "repo_inspect", "is_error": False},
        },
        {
            "type": "tool_call_result",
            "params": {
                "call_id": "search",
                "tool": "file_search",
                "is_error": False,
            },
        },
        {"type": "text_snapshot", "params": {"text": "A" * 80 + "."}},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertions = {item["name"]: item for item in accepted["assertions"]}
    assert assertions["repository_overview_starts_with_repo_inspect"]["passed"] is True
    assert assertions["repository_overview_waits_for_repo_inspect"]["passed"] is False
    assert assertions["repository_overview_waits_for_repo_inspect"][
        "premature_tools"
    ] == ["file_search"]


def test_provider_metrics_count_retries_and_errors():
    metrics = provider_metrics({
        "lines": [
            "Streaming inference unavailable; retrying attempt=1 provider_unavailable",
            "inference provider failed: provider_unavailable",
        ]
    })
    assert metrics["retry_attempts"] >= 1
    assert metrics["provider_error_markers"] >= 2


def test_event_acceptance_fails_closed_on_tool_error_and_truncated_markdown(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {"type": "tool_call_start", "params": {"call_id": "c1", "tool": "exec_command"}},
        {
            "type": "tool_call_result",
            "params": {
                "call_id": "c1",
                "tool": "exec_command",
                "is_error": True,
            },
        },
        {
            "type": "text_delta",
            "params": {"text": "`" + ("incomplete answer " * 8)},
        },
        {
            "type": "text_snapshot",
            "params": {"text": "`" + ("incomplete answer " * 8)},
        },
        {"type": "turn_done", "params": {}},
    ]
    path.write_text(
        "".join(__import__("json").dumps(record) + "\n" for record in records)
    )

    result = event_acceptance(str(path))

    assert result["tool_errors"] == [{"tool": "exec_command", "call_id": "c1"}]
    assert next(
        item for item in result["assertions"]
        if item["name"] == "tool_results_successful"
    )["passed"] is False
    assert next(
        item for item in result["assertions"]
        if item["name"] == "terminal_text_boundary"
    )["passed"] is False
    assert next(
        item for item in result["assertions"]
        if item["name"] == "markdown_backticks_balanced"
    )["passed"] is False


def test_event_acceptance_accepts_complete_markdown_table_boundary(tmp_path):
    path = tmp_path / "events.jsonl"
    text = (
        "这是已经完整结束的项目评审结果，表格为最后一节。"
        + ("证据充分。" * 12)
        + "\n\n| 维度 | 结果 |\n|---|---|\n| 风险 | 未关闭 |"
    )
    records = [
        {"type": "text_snapshot", "params": {"text": text}},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))

    result = event_acceptance(str(path))
    assertion = next(
        item for item in result["assertions"]
        if item["name"] == "terminal_text_boundary"
    )
    assert assertion["passed"] is True


def test_event_acceptance_accepts_complete_successful_turn(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {"type": "usage", "params": {}},
        {"type": "tool_call_start", "params": {"call_id": "c1", "tool": "file_read"}},
        {
            "type": "tool_call_result",
            "params": {"call_id": "c1", "tool": "file_read", "is_error": False},
        },
        {
            "type": "text_delta",
            "params": {"text": "A substantive, grounded answer with enough detail. " * 3},
        },
        {
            "type": "text_snapshot",
            "params": {"text": "A substantive, grounded answer with enough detail. " * 3},
        },
        {"type": "turn_done", "params": {}},
    ]
    path.write_text(
        "".join(__import__("json").dumps(record) + "\n" for record in records)
    )

    result = event_acceptance(str(path))

    assert result["inference_rounds"] == 1
    assert result["tool_calls"] == 1
    assert result["text_source"] == "text_snapshot"
    assert all(item["passed"] for item in result["assertions"])


def test_event_acceptance_uses_terminal_snapshot_after_dropped_delta(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {"type": "tool_call_start", "params": {"call_id": "c1", "tool": "file_read"}},
        {
            "type": "tool_call_result",
            "params": {"call_id": "c1", "tool": "file_read", "is_error": False},
        },
        {
            "type": "text_delta",
            "params": {"text": "broken 评分|------------|"},
        },
        {
            "type": "text_snapshot",
            "params": {"text": "A complete authoritative answer with grounded details. " * 3},
        },
        {"type": "turn_done", "params": {}},
    ]
    path.write_text(
        "".join(__import__("json").dumps(record) + "\n" for record in records)
    )

    result = event_acceptance(str(path))

    assert result["text_source"] == "text_snapshot"
    assert result["delta_text_chars"] < result["text_chars"]
    assert all(item["passed"] for item in result["assertions"])


def test_repository_overview_rejects_absence_claim_contradicted_by_entry_files(tmp_path):
    path = tmp_path / "events.jsonl"
    repo_output = json.dumps({
        "entry_files": [
            {"path": "README.md"},
            {"path": "docs/design/architecture-overview.md"},
        ],
        "missing_candidates": ["README", "ARCHITECTURE.md", "docs/architecture.md"],
    })
    records = [
        {"type": "tool_call_complete", "params": {
            "call_id": "repo", "tool": "repo_inspect", "args": {},
        }},
        {"type": "tool_call_result", "params": {
            "call_id": "repo", "tool": "repo_inspect", "is_error": False,
            "output": repo_output,
        }},
        {"type": "text_snapshot", "params": {
            "text": "项目文档不足：README 均缺失、无 docs/ 目录。" + "A" * 80,
        }},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertion = next(
        item for item in accepted["assertions"]
        if item["name"] == "repository_overview_presence_claims_match_evidence"
    )
    assert assertion["passed"] is False
    assert {item["evidence_path"] for item in assertion["conflicts"]} >= {
        "README.md", "docs/",
    }


def test_repository_overview_accepts_exact_missing_candidate_without_conflict(tmp_path):
    path = tmp_path / "events.jsonl"
    records = [
        {"type": "tool_call_complete", "params": {
            "call_id": "repo", "tool": "repo_inspect", "args": {},
        }},
        {"type": "tool_call_result", "params": {
            "call_id": "repo", "tool": "repo_inspect", "is_error": False,
            "output": json.dumps({
                "entry_files": [{"path": "README.md"}],
                "missing_candidates": ["ARCHITECTURE.md"],
            }),
        }},
        {"type": "text_snapshot", "params": {
            "text": "ARCHITECTURE.md is missing, while README.md is present." + "A" * 80 + ".",
        }},
        {"type": "turn_done", "params": {}},
    ]
    path.write_text("\n".join(json.dumps(record) for record in records))
    accepted = event_acceptance(str(path), require_repository_overview=True)
    assertion = next(
        item for item in accepted["assertions"]
        if item["name"] == "repository_overview_presence_claims_match_evidence"
    )
    assert assertion["passed"] is True
