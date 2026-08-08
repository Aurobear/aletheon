"""aletheon_diagnose — one-stop bundle: rendered TUI + daemon analyze + logs
+ audit tail, correlated into a single timeline.

Timeline alignment uses the raw ISO timestamp strings each source already
emits (daemon-log granularity); ISO-8601 strings sort chronologically, so no
parsing is required.
"""
import json
import os
import asyncio
import re
import unicodedata

from . import analyze as analyze_mod
from . import logs as logs_mod
from . import tui as tui_tools
from .health import systemd_user_environment


def _session_ids(sessions: list[dict]) -> set[str]:
    return {
        value
        for session in sessions
        if isinstance(session, dict)
        and isinstance((value := session.get("id", session.get("session_id"))), str)
    }


async def _wait_for_new_canonical_session(client, previous: set[str]) -> str | None:
    if client is None:
        return None
    for _ in range(40):
        sessions, error = await analyze_mod.canonical_sessions(client)
        if error is None:
            created = _session_ids(sessions) - previous
            if len(created) == 1:
                return next(iter(created))
            if len(created) > 1:
                return None
        await asyncio.sleep(0.25)
    return None


def _audit_path() -> str:
    return os.environ.get(
        "ALETHEON_AUDIT",
        os.path.expanduser("~/.local/state/aletheon/audit.jsonl"),
    )


INFRASTRUCTURE_ERRORS = (
    "provider_unavailable",
    "provider_rejected_request",
    "provider_timeout",
    "inference provider failed",
    "google_unauthorized_account",
    "Can't mount proc",
    "Permission denied",
    "Aletheon authorization failed",
)


def frame_assertions(frame: str, prompt_visible: bool,
                     forbidden_strings: list[str] | None = None) -> list[dict]:
    """Return semantic TUI assertions; prompt/input text alone is not success."""
    # Runtime failures are rendered by the TUI as a system line prefixed with
    # ``Error:``. Do not reject a legitimate repository-analysis answer merely
    # because it discusses a stable error code such as provider_unavailable.
    # Tool failures are checked independently from authoritative event records.
    error_lines = [
        line.lower()
        for line in frame.splitlines()
        if re.search(r"(^|\s)error\s*:", line, re.IGNORECASE)
    ]
    assertions = [
        {
            "name": f"forbidden:{text}",
            "passed": not any(text.lower() in line for line in error_lines),
        }
        for text in INFRASTRUCTURE_ERRORS
    ]
    assertions.extend(
        {
            "name": f"forbidden:{text}",
            "passed": text.lower() not in frame.lower(),
        }
        for text in dict.fromkeys(forbidden_strings or [])
    )
    infrastructure_clean = all(item["passed"] for item in assertions)
    assertions.append({
        "name": "final_answer",
        "passed": infrastructure_clean and len(frame.strip()) > 20,
    })
    assertions.append({"name": "prompt_returned", "passed": prompt_visible})
    return assertions


def provider_metrics(daemon_logs: dict) -> dict:
    lines = daemon_logs.get("lines", []) if isinstance(daemon_logs, dict) else []
    text = "\n".join(str(line) for line in lines)
    retry_attempts = len(re.findall(r"(?:retrying|attempt)", text, re.IGNORECASE))
    provider_errors = sum(
        text.lower().count(marker.lower()) for marker in INFRASTRUCTURE_ERRORS[:4]
    )
    return {
        "retry_attempts": retry_attempts,
        "provider_error_markers": provider_errors,
    }


def _repo_inspect_entry_paths(results: list[dict], call_id: str | None) -> list[str]:
    """Extract exact files proved present by the authoritative tool result."""
    if not call_id:
        return []
    record = next(
        (item for item in results if item.get("params", {}).get("call_id") == call_id),
        None,
    )
    output = record.get("params", {}).get("output") if record else None
    if not isinstance(output, str):
        return []
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return []
    entry_files = payload.get("entry_files", []) if isinstance(payload, dict) else []
    return [
        item["path"]
        for item in entry_files
        if isinstance(item, dict) and isinstance(item.get("path"), str)
    ]


def _contradicted_presence_claims(text: str, found_paths: list[str]) -> list[dict]:
    """Find absence claims contradicted by repo_inspect presence evidence."""
    aliases: dict[str, str] = {}
    for path in found_paths:
        normalized = path.replace("\\", "/").strip("/")
        if not normalized:
            continue
        aliases[normalized.casefold()] = path
        basename = normalized.rsplit("/", 1)[-1]
        aliases[basename.casefold()] = path
        if "." in basename:
            aliases[basename.rsplit(".", 1)[0].casefold()] = path
        if "/" in normalized:
            root = normalized.split("/", 1)[0]
            aliases[f"{root.casefold()}/"] = f"{root}/"
    strong_absence = re.compile(
        r"(?:\b(?:is|are)\s+(?:missing|absent|not\s+found)\b|"
        r"\bdoes\s+not\s+exist\b|不存在|缺少|缺失)",
        re.IGNORECASE,
    )
    prefix_absence = re.compile(r"(?:\b(?:no|without)\b|无|没有)", re.IGNORECASE)
    conflicts = []
    for clause in re.split(r"[\n。；;,，:：]+", text):
        folded = clause.casefold()
        strong = strong_absence.search(clause)
        prefixes = list(prefix_absence.finditer(clause))
        if not strong and not prefixes:
            continue
        for alias, evidence_path in aliases.items():
            alias_index = folded.find(alias)
            prefix_targets_alias = any(
                match.start() < alias_index and alias_index - match.end() <= 6
                for match in prefixes
            )
            if alias_index >= 0 and (strong or prefix_targets_alias):
                conflicts.append({
                    "claimed_absent": alias,
                    "evidence_path": evidence_path,
                    "clause": clause.strip()[:240],
                })
    return list({
        (item["claimed_absent"], item["evidence_path"], item["clause"]): item
        for item in conflicts
    }.values())


def _unsupported_staffing_inferences(text: str) -> list[str]:
    """Flag staffing-count conclusions that repository metadata cannot prove."""
    subject = re.compile(
        r"(?:maintainer|contributor|developer|team|staff(?:ing)?|"
        r"维护者|贡献者|开发者|团队|作者|项目)",
        re.IGNORECASE,
    )
    count = re.compile(
        r"(?:\b(?:single|solo|one[- ]person|one[- ]developer)\b|"
        r"单人|个人项目|一人|唯一(?:维护者|开发者)|只有一)",
        re.IGNORECASE,
    )
    disclaimer = re.compile(r"(?:do not infer|cannot establish|无法推断|不能证明)", re.IGNORECASE)
    return [
        clause.strip()[:240]
        for clause in re.split(r"[\n。；;]+", text)
        if subject.search(clause) and count.search(clause) and not disclaimer.search(clause)
    ]


def event_acceptance(path: str | None, require_repository_overview: bool = False) -> dict:
    """Check authoritative events for hidden tool and output failures."""
    summary = {
        "available": False,
        "event_count": 0,
        "inference_rounds": 0,
        "tool_calls": 0,
        "tool_errors": [],
        "text_source": None,
        "delta_text_chars": 0,
        "text_chars": 0,
        "assertions": [],
    }
    if not path or not os.path.isfile(path):
        return summary
    try:
        with open(path, encoding="utf-8") as handle:
            records = [json.loads(line) for line in handle if line.strip()]
    except (OSError, json.JSONDecodeError) as error:
        summary["assertions"].append({
            "name": "event_evidence_valid",
            "passed": False,
            "detail": type(error).__name__,
        })
        return summary

    summary["available"] = True
    summary["event_count"] = len(records)
    summary["inference_rounds"] = sum(
        record.get("type") == "usage" for record in records
    )
    starts = {
        record.get("params", {}).get("call_id")
        for record in records
        if record.get("type") == "tool_call_start"
    }
    results = [
        record for record in records
        if record.get("type") == "tool_call_result"
    ]
    result_ids = {
        record.get("params", {}).get("call_id") for record in results
    }
    summary["tool_calls"] = len(starts)
    summary["tool_errors"] = [
        {
            "tool": record.get("params", {}).get("tool"),
            "call_id": record.get("params", {}).get("call_id"),
        }
        for record in results
        if record.get("params", {}).get("is_error") is True
    ]
    completed_calls = [
        record.get("params", {})
        for record in records
        if record.get("type") == "tool_call_complete"
    ]
    delta_text = "".join(
        record.get("params", {}).get("text", "")
        for record in records
        if record.get("type") == "text_delta"
    )
    snapshots = [
        (
            index,
            record.get("params", {}).get("text", ""),
        )
        for index, record in enumerate(records)
        if record.get("type") == "text_snapshot"
    ]
    turn_done_indices = [
        index for index, record in enumerate(records)
        if record.get("type") == "turn_done"
    ]
    text = (snapshots[-1][1] if snapshots else delta_text).strip()
    summary["text_source"] = "text_snapshot" if snapshots else "text_delta"
    summary["delta_text_chars"] = len(delta_text.strip())
    summary["text_chars"] = len(text)
    last = text[-1:] or ""
    last_line = next(
        (line.strip() for line in reversed(text.splitlines()) if line.strip()),
        "",
    )
    complete_markdown_table_row = (
        last_line.startswith("|")
        and last_line.endswith("|")
        and last_line.count("|") >= 3
    )
    terminal_boundary = bool(
        last
        and (
            unicodedata.category(last).startswith("P")
            or last in ")]}）】』」"
            or complete_markdown_table_row
        )
    )
    summary["assertions"] = [
        {
            "name": "event_turn_done",
            "passed": bool(turn_done_indices),
        },
        {
            "name": "authoritative_text_snapshot",
            "passed": (
                len(snapshots) == 1
                and bool(turn_done_indices)
                and snapshots[0][0] < turn_done_indices[-1]
            ),
            "count": len(snapshots),
        },
        {
            "name": "tool_results_complete",
            "passed": bool(starts) and starts == result_ids,
            "started": len(starts),
            "result_count": len(result_ids),
        },
        {
            "name": "tool_results_successful",
            "passed": not summary["tool_errors"],
            "errors": summary["tool_errors"],
        },
        {
            "name": "terminal_text_substantive",
            "passed": len(text) >= 80,
            "chars": len(text),
        },
        {
            "name": "terminal_text_boundary",
            "passed": terminal_boundary,
            "last_character": last,
        },
        {
            "name": "markdown_backticks_balanced",
            "passed": text.count("`") % 2 == 0,
            "count": text.count("`"),
        },
        {
            "name": "terminal_text_encoding",
            "passed": "\ufffd" not in text,
        },
    ]
    if require_repository_overview:
        first_tool = completed_calls[0].get("tool") if completed_calls else None
        first_repo_call = next(
            (call for call in completed_calls if call.get("tool") == "repo_inspect"),
            None,
        )
        first_repo_call_id = (
            first_repo_call.get("call_id") if first_repo_call else None
        )
        found_entry_paths = _repo_inspect_entry_paths(results, first_repo_call_id)
        presence_conflicts = _contradicted_presence_claims(text, found_entry_paths)
        staffing_inferences = _unsupported_staffing_inferences(text)
        calls_before_repo = (
            completed_calls[:completed_calls.index(first_repo_call)]
            if first_repo_call in completed_calls
            else completed_calls
        )
        entry_phase_only = all(
            call.get("tool") == "file_read" for call in calls_before_repo
        )
        first_repo_result_index = next(
            (
                index
                for index, record in enumerate(records)
                if record.get("type") == "tool_call_result"
                and record.get("params", {}).get("call_id") == first_repo_call_id
            ),
            None,
        )
        premature_discovery = [
            record.get("params", {}).get("tool")
            for index, record in enumerate(records)
            if first_repo_result_index is not None
            and index < first_repo_result_index
            and record.get("type") == "tool_call_complete"
            and record.get("params", {}).get("call_id") != first_repo_call_id
            and record.get("params", {}).get("tool") != "file_read"
        ]
        broad_globs = []
        for call in completed_calls:
            if call.get("tool") != "glob":
                continue
            args = call.get("args", {})
            patterns = args.get("patterns", []) if isinstance(args, dict) else []
            if isinstance(patterns, list) and len(patterns) > 6:
                broad_globs.extend(
                    pattern for pattern in patterns if isinstance(pattern, str)
                )
            broad_globs.extend(
                pattern
                for pattern in patterns
                if isinstance(pattern, str)
                and (
                    "**" in pattern
                    or any(
                        any(marker in segment for marker in ("*", "?", "["))
                        for segment in pattern.replace("\\", "/").split("/")[:-1]
                    )
                )
            )
        broad_globs = list(dict.fromkeys(broad_globs))
        summary["assertions"].extend([
            {
                "name": "repository_overview_starts_with_repo_inspect",
                "passed": first_repo_call is not None and entry_phase_only,
                "first_tool": first_tool,
            },
            {
                "name": "repository_overview_waits_for_repo_inspect",
                "passed": (
                    first_repo_result_index is not None
                    and not premature_discovery
                ),
                "premature_tools": premature_discovery,
            },
            {
                "name": "repository_overview_avoids_broad_glob",
                "passed": not broad_globs,
                "patterns": broad_globs,
            },
            {
                "name": "repository_overview_presence_claims_match_evidence",
                "passed": not presence_conflicts,
                "found_entry_paths": found_entry_paths,
                "conflicts": presence_conflicts,
            },
            {
                "name": "repository_overview_avoids_staffing_inference",
                "passed": not staffing_inferences,
                "claims": staffing_inferences,
            },
        ])
    return summary


def _audit_tail(n: int = 20) -> list[str]:
    try:
        with open(_audit_path(), encoding="utf-8", errors="replace") as f:
            return [ln.rstrip("\n") for ln in f.readlines()[-n:]]
    except OSError:
        return []


def build_timeline(journal: list[dict], audit_lines: list[str]) -> list[dict]:
    """Merge journal events and audit JSONL lines into one ts-sorted list."""
    events: list[dict] = []
    for ev in journal or []:
        ts = ev.get("timestamp") or ev.get("ts") or ""
        events.append({
            "ts": ts, "source": "journal",
            "summary": ev.get("type", ev.get("event", "event")),
        })
    for line in audit_lines or []:
        try:
            rec = json.loads(line)
        except (json.JSONDecodeError, TypeError):
            continue
        ts = rec.get("timestamp", "")
        tool = rec.get("tool_name", "tool")
        err = " [error]" if rec.get("is_error") else ""
        events.append({"ts": ts, "source": "audit",
                       "summary": f"{tool}{err}"})
    events.sort(key=lambda e: e["ts"])
    return events


async def diagnose(client, task: str, settle_secs: float = 6.0,
                   timeout: float = 120.0, cols: int = 120,
                   rows: int = 50, working_dir: str | None = None,
                   expected_cwd: str | None = None,
                   forbidden_strings: list[str] | None = None,
                   require_repository_overview: bool = False) -> dict:
    """Drive the TUI with `task`, capture the settled frame, and bundle it
    with daemon-side analysis, logs, audit tail, and a merged timeline.

    Completion requires the durable TUI recorder's authoritative `turn_done`
    event plus a stable frame whose input prompt is visible and no busy spinner
    remains. The event stream is also checked for failed/missing tool results
    and structurally incomplete terminal text."""
    # Phase 0: launch the TUI (ready-gated) WITHOUT sending the task yet.
    # NOTE: ratatui uses the terminal ALTERNATE screen (no tmux scrollback),
    # and the TUI keeps its own internal scroll, auto-scrolling to the input
    # prompt when a turn ends. So a long answer can be scrolled off the pane by
    # completion time and a single capture-pane only sees the tail. A taller
    # pane (`rows`) helps short/medium answers fit but does NOT fully solve
    # long ones — that needs scroll-and-stitch or a TUI export mode (follow-up).
    cwd = os.path.realpath(working_dir or os.getcwd())
    started = await tui_tools.tui_start(task="", cols=cols, rows=rows,
                                        working_dir=cwd)
    if not started.get("ok"):
        return {"error": "tui_start failed", "detail": started}

    selected_session_id = None
    sessions_before: set[str] = set()
    if client is not None:
        sessions, error = await analyze_mod.canonical_sessions(client)
        if error is None:
            sessions_before = _session_ids(sessions)

    try:
        # Test runs must not inherit a previously failed or structurally
        # incomplete conversation. The production TUI resumes the most recent
        # workspace session by default, so explicitly create a clean session
        # before establishing the task baseline.
        await tui_tools.tui_send("/new", submit=True)
        await tui_tools.tui_capture(
            scrollback=True, wait_stable=True, stable_secs=0.8, timeout=20.0,
        )
        selected_session_id = await _wait_for_new_canonical_session(
            client, sessions_before
        )

        # Phase 1: submit the task and let the input echo settle -> baseline.
        # This baseline includes the user's echoed input but no response yet.
        await tui_tools.tui_send(task, submit=True)
        submitted = await tui_tools.tui_capture(
            scrollback=True, wait_stable=True, stable_secs=0.8, timeout=20.0,
        )
        baseline_frame = submitted.get("frame", "")

        # Phase 2: wait for the assistant response to appear BEYOND the
        # submitted baseline and then stay quiet for settle_secs (heuristic
        # completion — see the docstring caveat).
        cap = await tui_tools.tui_wait_turn_done(
            baseline=started.get("turn_done_count", 0), timeout=timeout
        )
    finally:
        await tui_tools.tui_stop()

    if selected_session_id:
        daemon_analyze = await analyze_mod.analyze(
            client, session_id=selected_session_id
        )
    else:
        daemon_analyze = await analyze_mod.analyze(client)
    daemon_logs = await logs_mod.logs(client, last_n=50)
    audit_tail = _audit_tail()

    recent_journal = []
    if isinstance(daemon_analyze, dict):
        recent_journal = daemon_analyze.get("recent_journal", []) or []

    verdict = "pass"
    if cap.get("checks"):
        verdict = "fail"
    if isinstance(daemon_analyze, dict) and daemon_analyze.get("healthy") is False:
        verdict = "fail"
    if cap.get("turn_done") is not True or cap.get("stable") is False:
        verdict = "fail"

    assertions = []
    frame = cap.get("frame", "")
    expected = os.path.realpath(expected_cwd) if expected_cwd else None
    if expected:
        actual_cwd = os.path.realpath(started.get("working_dir", ""))
        assertions.append({"name": "expected_cwd", "passed": actual_cwd == expected,
                           "actual": actual_cwd,
                           "expected": expected})
    assertions.extend(frame_assertions(
        frame,
        cap.get("prompt_visible") is True,
        forbidden_strings,
    ))
    event_summary = event_acceptance(
        cap.get("event_path"),
        require_repository_overview=require_repository_overview,
    )
    assertions.extend(event_summary["assertions"])
    if any(not item["passed"] for item in assertions):
        verdict = "fail"

    async def command(*args: str, env: dict[str, str] | None = None) -> str:
        try:
            proc = await asyncio.create_subprocess_exec(
                *args, cwd=cwd, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT, env=env)
            out, _ = await asyncio.wait_for(proc.communicate(), timeout=5)
            return out.decode("utf-8", "replace").strip()
        except (OSError, asyncio.TimeoutError):
            return ""

    socket_path = getattr(
        client,
        "socket_path",
        f"/run/user/{os.getuid()}/aletheon/aletheon.sock",
    )
    user_manager_env = systemd_user_environment(socket_path)
    preflight = {
        "working_dir": cwd,
        "git_commit": await command("git", "rev-parse", "HEAD"),
        "git_toplevel": await command("git", "rev-parse", "--show-toplevel"),
        "binary": await command("aletheon", "version"),
        "service_active": await command(
            "systemctl", "--user", "is-active", "aletheon.service",
            env=user_manager_env),
        "service_started": await command(
            "systemctl", "--user", "show", "aletheon.service",
            "-p", "ActiveEnterTimestamp", env=user_manager_env),
    }

    return {
        "task": task,
        "session_id": selected_session_id,
        "rendered_frame": cap.get("frame", ""),
        "stable": cap.get("stable"),
        "prompt_visible": cap.get("prompt_visible"),
        "turn_done_count": cap.get("turn_done_count"),
        "completion_source": cap.get("completion_source"),
        "event_path": cap.get("event_path"),
        "event_evidence": cap.get("event_evidence"),
        "event_acceptance": event_summary,
        "completion": (
            "authoritative client_event:turn_done with durable event evidence"
            if cap.get("turn_done") is True
            else f"heuristic (settled {settle_secs}s beyond input echo; may be mid-turn)"
        ),
        "tui_checks": cap.get("checks", []),
        "daemon": {"analyze": daemon_analyze, "logs": daemon_logs},
        "audit_tail": audit_tail,
        "timeline": build_timeline(recent_journal, audit_tail),
        "provider_metrics": provider_metrics(daemon_logs),
        "preflight": preflight,
        "assertions": assertions,
        "verdict": verdict,
    }
