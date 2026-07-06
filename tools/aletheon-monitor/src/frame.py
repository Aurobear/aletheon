"""frame.py — pure functions to normalize and compare rendered TUI frames.

No IO. Given raw `tmux capture-pane` text, produce a stable canonical form
and answer "did the screen stop changing?".
"""
import re

# Matches CSI / most ANSI escape sequences.
_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")


def normalize(raw: str) -> str:
    """Strip ANSI escapes, right-trim each line, drop trailing blank lines."""
    text = _ANSI_RE.sub("", raw)
    lines = [line.rstrip() for line in text.splitlines()]
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def is_stable(frames: list[str], window: int = 3) -> bool:
    """True when the last `window` frames are all identical (screen settled)."""
    if len(frames) < window:
        return False
    tail = frames[-window:]
    return all(f == tail[0] for f in tail)


def changed(a: str, b: str) -> bool:
    """True when two raw frames differ after normalization."""
    return normalize(a) != normalize(b)
