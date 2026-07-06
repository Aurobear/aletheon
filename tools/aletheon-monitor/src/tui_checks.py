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
