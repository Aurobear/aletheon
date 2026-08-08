"""Render local issue drafts without granting the model GitHub authority."""

from __future__ import annotations

import shlex
from collections.abc import Mapping


def render_issue_draft(
    result: Mapping[str, object],
    cluster: Mapping[str, object],
    analysis: Mapping[str, object] | None = None,
) -> str:
    fingerprint = result.get("fingerprint", "unavailable")
    process = (
        result.get("process") if isinstance(result.get("process"), Mapping) else {}
    )
    summary = (
        analysis.get("summary")
        if isinstance(analysis, Mapping) and analysis.get("status") == "completed"
        else "DeepSeek analysis was unavailable or not valid."
    )
    confidence = analysis.get("confidence") if isinstance(analysis, Mapping) else None
    lines = [
        f"# [Nightwatch] {result['case_id']} — {result['failure_class']}",
        "",
        "## Deterministic result",
        "",
        f"- Fingerprint: `{fingerprint}`",
        f"- Outcome: `{result['outcome']}`",
        f"- Commit: `{result['source']['commit_sha']}`",
        f"- Run: `{result['run_id']}`",
        f"- Occurrences: `{cluster.get('occurrence_count', 1)}`",
        f"- Exit: `{process.get('exit_code')}`",
        f"- Timed out: `{process.get('timed_out')}`",
        f"- Process group reaped: `{process.get('process_group_reaped')}`",
        "",
        "## Reproduction",
        "",
        "```text",
        shlex.join(str(item) for item in process.get("argv", [])),
        "```",
        "",
        "## Model-authored diagnosis (not the verdict)",
        "",
        str(summary),
    ]
    if confidence is not None:
        lines.extend(("", f"Confidence: `{confidence}`"))
    lines.extend(
        (
            "",
            "## Evidence",
            "",
            "The sealed bundle path and digests are recorded in the Nightwatch run database.",
            "",
        )
    )
    return "\n".join(lines)
