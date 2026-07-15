"""tui_checks.py — pure assertions over a normalized TUI frame.

Each check returns a list of finding dicts:
    {"kind": str, "severity": "low|medium|high", "evidence": str, "detail"?: str}
Empty list == check passed.
"""


def check_dup_render(frame: str, min_block: int = 3) -> list[dict]:
    """Detect a run of >= `min_block` consecutive non-blank lines that appears
    at least twice (non-overlapping) in the frame. O(n) via sliding window."""
    lines = frame.splitlines()
    n = len(lines)
    seen: dict[tuple, int] = {}
    for i in range(n - min_block + 1):
        window = tuple(lines[i:i + min_block])
        if all(w.strip() == "" for w in window):
            continue  # ignore all-blank windows (padding, not real content)
        if window in seen:
            prev = seen[window]
            if i - prev >= min_block:  # non-overlapping duplicate
                return [{
                    "kind": "dup_render",
                    "severity": "high",
                    "evidence": "\n".join(window)[:400],
                    "detail": f"{min_block}-line block repeated at lines "
                              f"{prev + 1} and {i + 1}",
                }]
        else:
            seen[window] = i
    return []


import re

_MD_SEP_RE = re.compile(r"\|\s*-{2,}")  # markdown separator row: |--- or | ---
_UNKNOWN_SKILL_RE = re.compile(r"未知技能:\s*/\S+")


def check_raw_markdown(frame: str) -> list[dict]:
    """Unrendered markdown table pipes leaking into the TUI."""
    for ln in frame.splitlines():
        if _MD_SEP_RE.search(ln) or "||" in ln:
            return [{
                "kind": "raw_markdown",
                "severity": "medium",
                "evidence": ln.strip()[:200],
            }]
    return []


def check_double_reflection(frame: str) -> list[dict]:
    """The 'Reflection: Reflection:' double-prefix bug (tui/response.rs:212)."""
    if "Reflection: Reflection:" in frame:
        return [{
            "kind": "double_reflection",
            "severity": "low",
            "evidence": "Reflection: Reflection:",
        }]
    return []


def check_unknown_skill_path(frame: str) -> list[dict]:
    """Absolute path mis-parsed as a slash command (tui/app/submit.rs:25)."""
    m = _UNKNOWN_SKILL_RE.search(frame)
    if m:
        return [{
            "kind": "unknown_skill_path",
            "severity": "medium",
            "evidence": m.group(0)[:200],
        }]
    return []


def check_permission_denied(frame: str) -> list[dict]:
    """Sandbox / filesystem permission failures surfaced in the TUI."""
    for ln in frame.splitlines():
        if "Permission denied" in ln:
            return [{
                "kind": "permission_denied",
                "severity": "high",
                "evidence": ln.strip()[:200],
            }]
    return []


def check_known_tool_failures(frame: str) -> list[dict]:
    """Known infrastructure failures must never be accepted as an answer."""
    needles = (
        "google_unauthorized_account",
        "Can't mount proc",
        "Aletheon authorization failed",
    )
    for needle in needles:
        if needle in frame:
            return [{
                "kind": "known_tool_failure",
                "severity": "high",
                "evidence": needle,
            }]
    return []


_CHECKS = [
    check_dup_render,
    check_raw_markdown,
    check_double_reflection,
    check_unknown_skill_path,
    check_permission_denied,
    check_known_tool_failures,
]


def run_checks(frame: str) -> list[dict]:
    """Run every render check over a frame; concatenate findings."""
    findings: list[dict] = []
    for fn in _CHECKS:
        findings.extend(fn(frame))
    return findings
