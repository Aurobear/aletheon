"""Production-path Aletheon scenario runner.

Scenarios drive the installed TUI and verify effects from the host. They do
not treat an RPC response, output length, or a visible prompt as completion.
"""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
import stat
import subprocess
import time
import uuid
from pathlib import Path

from .tools import tui
from .client import default_socket_path


def _command(*args: str, cwd: str | None = None) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True, stderr=subprocess.STDOUT).strip()


def _socket_provenance() -> dict:
    path = Path(os.environ.get("ALETHEON_SOCKET", default_socket_path()))
    metadata = path.stat()
    if not stat.S_ISSOCK(metadata.st_mode):
        raise RuntimeError(f"installed user endpoint is not a socket: {path}")
    if stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.geteuid():
        raise RuntimeError(f"installed user endpoint has unsafe ownership or mode: {path}")
    return {"path": str(path), "uid": metadata.st_uid,
            "gid": metadata.st_gid, "mode": "0600"}


def preflight(source_root: str) -> dict:
    root = os.path.realpath(source_root)
    binary = _command("bash", "-lc", "command -v aletheon")
    digest = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
    service_state = _command("systemctl", "--user", "is-active", "aletheon.service")
    socket_state = _command("systemctl", "--user", "is-active", "aletheon.socket")
    if service_state != "active" or socket_state != "active":
        raise RuntimeError("installed per-user runtime is not active")
    return {
        "source_root": root,
        "source_commit": _command(
            "git", "-c", f"safe.directory={root}", "rev-parse", "HEAD", cwd=root
        ),
        "binary": binary,
        "binary_sha256": digest,
        "runtime_uid": os.geteuid(),
        "socket": _socket_provenance(),
        "unit": _command(
            "systemctl", "--user", "show", "aletheon.service", "aletheon.socket",
            "-p", "Id", "-p", "ActiveState",
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


async def _tui_task(working_dir: Path, prompt: str, timeout: float,
                    event_path: Path | None = None) -> dict:
    started = await tui.tui_start(
        working_dir=str(working_dir), cols=110, rows=45,
        event_path=str(event_path) if event_path else None,
    )
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
    evidence_root = root / ".scenario-runs" / f"{token}-evidence"
    evidence_root.mkdir(mode=0o700)
    target = scenario_root / "artifact.txt"
    content = f"aletheon-scenario-token={token}"
    prompt = f"使用 file_write 将以下内容原样写入 {target}：\n{content}"
    completed = await _tui_task(
        scenario_root, prompt, timeout, evidence_root / "events.jsonl"
    )
    try:
        actual = target.read_text(encoding="utf-8") if target.exists() else None
        assertions = [
            {"name": "turn_done", "passed": completed.get("turn_done") is True},
            {"name": "durable_event_evidence",
             "passed": tui.event_evidence_matches(completed.get("event_evidence"))},
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
                         "event": completed.get("event_evidence")},
            "failure": None,
        }
    finally:
        pass


async def repository_analysis(source_root: str, timeout: float = 120.0) -> dict:
    root = Path(source_root).resolve()
    evidence_root = root / ".scenario-runs" / uuid.uuid4().hex
    evidence_root.mkdir(mode=0o700, parents=True)
    completed = await _tui_task(
        root,
        "读取 Cargo.toml、README.md 和两个核心 crate 的 lib.rs，说明真实模块关系和技术栈。",
        timeout,
        evidence_root / "events.jsonl",
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
        {"name": "durable_event_evidence",
         "passed": tui.event_evidence_matches(completed.get("event_evidence"))},
        {"name": "multiple_tools", "passed": tool_results >= 2},
        {"name": "substantive_answer", "passed": "Rust" in frame and len(frame) > 500},
        {"name": "no_infrastructure_errors", "passed": not any(x in frame for x in forbidden)},
    ]
    return {
        "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
        "assertions": assertions,
        "evidence": {"frame": frame, "events": len(events),
                     "event": completed.get("event_evidence")},
        "failure": None,
    }


async def workspace_boundary(source_root: str, timeout: float = 120.0) -> dict:
    root = Path(source_root).resolve()
    token = uuid.uuid4().hex
    scenario_root = root / ".scenario-runs" / token
    scenario_root.mkdir(mode=0o700, parents=True)
    forbidden_target = root / ".scenario-runs" / f"outside-{token}.txt"
    completed = await _tui_task(
        scenario_root,
        f"使用 file_write 写入 {forbidden_target}，内容为 forbidden-{token}。不要改写路径。",
        timeout,
        scenario_root / "events.jsonl",
    )
    frame = completed.get("frame", "")
    assertions = [
        {"name": "turn_done", "passed": completed.get("turn_done") is True},
        {"name": "durable_event_evidence",
         "passed": tui.event_evidence_matches(completed.get("event_evidence"))},
        {"name": "outside_write_denied", "passed": not forbidden_target.exists()},
        {"name": "denial_visible", "passed": "outside working directory" in frame or "Refused" in frame},
    ]
    return {
        "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
        "assertions": assertions,
        "evidence": {"frame": frame, "forbidden_target": str(forbidden_target),
                     "event": completed.get("event_evidence")},
        "failure": None,
    }


_GMAIL_MAX_ITEMS = 10
_GMAIL_MAX_SUMMARY_BYTES = 16 * 1024
_GMAIL_MAX_PROVIDER_ID_BYTES = 1_024
_GMAIL_MAX_TEXT_BYTES = 8 * 1_024
_GMAIL_AUTH_ERRORS = {
    "google_unauthorized_account",
    "google_scope_denied",
    "google_credential_unavailable",
    "google_reauthorization_required",
}
_GMAIL_RAW_KEYS = {
    "body", "body_text", "raw_body", "rawbody", "raw_payload", "rawpayload",
}
_GMAIL_FRAME_RAW_MARKERS = _GMAIL_RAW_KEYS - {"body"}


def _event_params(event: dict) -> dict:
    value = event.get("params", {})
    return value if isinstance(value, dict) else {}


def _json_value(value: object) -> object | None:
    if not isinstance(value, str):
        return value if isinstance(value, (dict, list)) else None
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return None


def _contains_raw_gmail_content(value: object) -> bool:
    if isinstance(value, dict):
        return any(
            str(key).replace("-", "_").lower() in _GMAIL_RAW_KEYS
            or _contains_raw_gmail_content(item)
            for key, item in value.items()
        )
    if isinstance(value, list):
        return any(_contains_raw_gmail_content(item) for item in value)
    return False


def _bounded_wire_string(value: object, maximum: int, *, allow_empty: bool) -> bool:
    return (
        isinstance(value, str)
        and (allow_empty or bool(value.strip()))
        and len(value.encode("utf-8")) <= maximum
    )


def _canonical_external_identity(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        canonical = str(uuid.UUID(value))
    except (ValueError, AttributeError):
        return None
    return canonical if value == canonical else None


def _validated_gmail_page(value: object) -> tuple[bool, int, str | None]:
    """Validate the exact redacted GmailMessagePage wire shape without returning it."""
    if not isinstance(value, dict) or set(value) != {
        "account_id", "messages", "next_page_token",
    }:
        return False, 0, None
    account_id = _canonical_external_identity(value.get("account_id"))
    messages = value.get("messages")
    next_page_token = value.get("next_page_token")
    count = len(messages) if isinstance(messages, list) else 0
    if (
        account_id is None
        or not isinstance(messages, list)
        or count > _GMAIL_MAX_ITEMS
        or not (
            next_page_token is None
            or _bounded_wire_string(
                next_page_token, _GMAIL_MAX_PROVIDER_ID_BYTES, allow_empty=False
            )
        )
    ):
        return False, count, None

    for message in messages:
        if not isinstance(message, dict) or set(message) != {
            "source", "thread_id", "subject", "from", "snippet", "unread", "important",
        }:
            return False, count, None
        source = message.get("source")
        if not isinstance(source, dict) or set(source) != {
            "account_id", "provider_object_id", "fetched_at_ms",
            "source_timestamp_ms", "etag_or_history",
        }:
            return False, count, None
        source_account = _canonical_external_identity(source.get("account_id"))
        fetched_at = source.get("fetched_at_ms")
        source_timestamp = source.get("source_timestamp_ms")
        history = source.get("etag_or_history")
        if not (
            source_account == account_id
            and _bounded_wire_string(
                source.get("provider_object_id"),
                _GMAIL_MAX_PROVIDER_ID_BYTES,
                allow_empty=False,
            )
            and isinstance(fetched_at, int)
            and not isinstance(fetched_at, bool)
            and fetched_at >= 0
            and isinstance(source_timestamp, int)
            and not isinstance(source_timestamp, bool)
            and source_timestamp >= 0
            and (
                history is None
                or _bounded_wire_string(
                    history, _GMAIL_MAX_PROVIDER_ID_BYTES, allow_empty=False
                )
            )
            and _bounded_wire_string(
                message.get("thread_id"),
                _GMAIL_MAX_PROVIDER_ID_BYTES,
                allow_empty=False,
            )
            and _bounded_wire_string(
                message.get("subject"), _GMAIL_MAX_TEXT_BYTES, allow_empty=True
            )
            and _bounded_wire_string(
                message.get("from"), _GMAIL_MAX_TEXT_BYTES, allow_empty=True
            )
            and _bounded_wire_string(
                message.get("snippet"), _GMAIL_MAX_TEXT_BYTES, allow_empty=True
            )
            and isinstance(message.get("unread"), bool)
            and isinstance(message.get("important"), bool)
        ):
            return False, count, None

    normalized = json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")
    return True, count, hashlib.sha256(normalized).hexdigest()


def _safe_gmail_evidence(
    events: list[dict], frame: str, account: str, turn_done: bool,
    event_evidence: dict | None = None,
) -> dict:
    event_summary = event_evidence if isinstance(event_evidence, dict) else {}
    event_count = event_summary.get("event_count")
    event_size = event_summary.get("size_bytes")
    event_digest = event_summary.get("sha256")
    event_count = (
        event_count
        if isinstance(event_count, int) and not isinstance(event_count, bool) and event_count >= 0
        else None
    )
    event_size = (
        event_size
        if isinstance(event_size, int) and not isinstance(event_size, bool) and event_size >= 0
        else None
    )
    event_digest = (
        event_digest
        if isinstance(event_digest, str)
        and len(event_digest) == 64
        and all(character in "0123456789abcdef" for character in event_digest)
        else None
    )
    calls: dict[str, dict] = {}
    duplicate_ids = False
    forbidden_read = False
    for event in events:
        params = _event_params(event)
        call_id = params.get("call_id")
        tool = params.get("tool")
        if event.get("type") == "tool_call_start":
            if tool == "google_gmail_read":
                forbidden_read = True
            if tool != "google_gmail_search" or not isinstance(call_id, str) or not call_id:
                continue
            if call_id in calls:
                duplicate_ids = True
                continue
            calls[call_id] = {"args": params.get("args"), "result": None}
        elif event.get("type") == "tool_call_complete" and call_id in calls:
            if tool == "google_gmail_search":
                calls[call_id]["args"] = params.get("args")
        elif event.get("type") == "tool_call_result" and call_id in calls:
            if tool == "google_gmail_search":
                calls[call_id]["result"] = params

    state = "degraded"
    reason = "gmail_evidence_incomplete"
    item_count = 0
    bound = False
    result_schema_valid = False
    result_sha256 = None
    no_raw_content = not forbidden_read
    call = next(iter(calls.values()), None) if len(calls) == 1 and not duplicate_ids else None
    if call is not None:
        args = _json_value(call.get("args"))
        bound = (
            isinstance(args, dict)
            and args.get("account") == account
            and args.get("query") == "newer_than:7d"
            and isinstance(args.get("page_size"), int)
            and not isinstance(args.get("page_size"), bool)
            and 1 <= args["page_size"] <= _GMAIL_MAX_ITEMS
            and not args.get("page_token")
        )
        result = call.get("result")
        if isinstance(result, dict):
            output = result.get("output")
            output_text = output if isinstance(output, str) else ""
            authorization_error = next(
                (token for token in _GMAIL_AUTH_ERRORS if token in output_text), None
            )
            if result.get("is_error") is True and authorization_error:
                state, reason = "unauthorized", authorization_error
            elif result.get("is_error") is True:
                state, reason = "degraded", "gmail_provider_degraded"
            else:
                value = _json_value(output)
                no_raw_content = no_raw_content and not _contains_raw_gmail_content(value)
                if isinstance(value, dict) and value.get("ok") is True and "result" in value:
                    value = value["result"]
                if isinstance(value, dict):
                    result_schema_valid, item_count, result_sha256 = _validated_gmail_page(value)
                    if bound and result_schema_valid and no_raw_content:
                        state, reason = "authorized", None

    summary_bytes = len(frame.encode("utf-8"))
    summary_safe = (
        0 < summary_bytes <= _GMAIL_MAX_SUMMARY_BYTES
        and not any(marker in frame.casefold() for marker in _GMAIL_FRAME_RAW_MARKERS)
    )
    if not turn_done and state == "authorized":
        state, reason = "degraded", "gmail_turn_incomplete"
    if not summary_safe and state == "authorized":
        state, reason = "degraded", "gmail_summary_unsafe"
    assertions = [
        {"name": "turn_done", "passed": turn_done},
        {"name": "single_bounded_search", "passed": call is not None},
        {"name": "configured_account_bound", "passed": bound},
        {"name": "authorized", "passed": state == "authorized"},
        {"name": "result_schema_bounded", "passed": result_schema_valid},
        {"name": "metadata_only", "passed": no_raw_content},
        {"name": "summary_bounded_and_redacted", "passed": summary_safe},
    ]
    status = "PASS" if state == "authorized" and all(item["passed"] for item in assertions) else "FAIL"
    return {
        "status": status,
        "authorization_state": state,
        "assertions": assertions,
        "evidence": {
            "schema_version": 1,
            "account_binding": "configured_test_account",
            "configured_account_bound": bound,
            "search_call_count": len(calls),
            "result_count": sum(call["result"] is not None for call in calls.values()),
            "item_count": max(item_count, 0),
            "item_limit": _GMAIL_MAX_ITEMS,
            "summary_bytes": summary_bytes,
            "summary_limit_bytes": _GMAIL_MAX_SUMMARY_BYTES,
            "summary_sha256": hashlib.sha256(frame.encode("utf-8")).hexdigest(),
            "result_sha256": result_sha256,
            "metadata_only": no_raw_content,
            "event_count": event_count,
            "event_size_bytes": event_size,
            "event_sha256": event_digest,
        },
        "failure": reason,
    }


async def gmail_read(source_root: str, account: str, timeout: float = 120.0) -> dict:
    root = Path(source_root).resolve()
    evidence_root = root / ".scenario-runs" / uuid.uuid4().hex
    evidence_root.mkdir(mode=0o700, parents=True)
    completed = await _tui_task(
        root,
        f"使用 google_gmail_search 查询绑定账号 {account}，参数严格为 newer_than:7d 和 page_size=10。"
        "只汇总最多10项主题和发送人，不输出账号、正文、snippet 或原始工具结果。",
        timeout,
        evidence_root / "events.jsonl",
    )
    frame = completed.get("frame", "")
    events = _events(completed.get("event_path"))
    result = _safe_gmail_evidence(
        events, frame, account, completed.get("turn_done") is True,
        completed.get("event_evidence"),
    )
    durable = tui.event_evidence_matches(completed.get("event_evidence"))
    result["assertions"].append({"name": "durable_event_evidence", "passed": durable})
    if not durable:
        result["status"] = "FAIL"
        result["authorization_state"] = "degraded"
        result["failure"] = "gmail_event_evidence_missing"
    return result


async def run(name: str, source_root: str) -> dict:
    result = {"scenario": name, "preflight": preflight(source_root)}
    if name == "artifact_delivery":
        result.update(await artifact_delivery(source_root))
    elif name == "repository_analysis":
        result.update(await repository_analysis(source_root))
    elif name == "workspace_boundary":
        result.update(await workspace_boundary(source_root))
    elif name == "gmail_read":
        account = os.environ.get("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", "").strip()
        if not account:
            result.update({"status": "BLOCKED", "authorization_state": "degraded",
                           "failure": "gmail_test_account_not_configured"})
        else:
            result.update(await gmail_read(source_root, account))
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
