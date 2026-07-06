# Aletheon TUI Observability + Automated Debug-Loop — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Claude a real-TUI capture layer (tmux) plus a one-stop `aletheon_diagnose` bundle, so the `aletheon-tester` skill can drive aletheon and diagnose it from **logs AND actual rendered TUI**.

**Architecture:** Three pure/near-pure Python modules inside `tools/aletheon-monitor/src/` — `tui_session.py` (tmux lifecycle), `frame.py` (normalize/stability, pure), `tui_checks.py` (render assertions, pure) — wrapped by thin MCP tools in `src/tools/tui.py` and `src/tools/diagnose.py`, then registered in the existing FastMCP `server.py` alongside the current 9 tools. The `aletheon-tester` SKILL.md gains a "TUI track".

**Tech Stack:** Python 3.10+, `mcp` SDK (existing), `tmux` 3.7b (installed), `pytest` (new dev dep). No new runtime deps.

**Scope this round:** Observability + automation ONLY. The catalogued T/D/I bugs (dup-render, daemon perms, timeout, /dev/null, slash-parse, session persistence, audit session_id) are OUT of scope — they get fixed next round using this tooling.

**Design ref:** `docs/plans/2026-07-06-tui-observability-debug-loop-design.md`

**Conventions to follow (from existing monitor code):**
- Tool modules live in `src/tools/<name>.py`; each MCP-facing function is `async def`.
- `server.py` holds a `TOOLS` list (`mcp.types.Tool`) and a `_HANDLERS` dict (`name -> lambda client, args: <coroutine>`); dispatch does `await handler(get_client(), arguments)`.
- Async subprocess pattern is `asyncio.create_subprocess_exec(..., stdout=PIPE, stderr=PIPE)` then `await asyncio.wait_for(proc.communicate(), timeout=...)` (see `src/tools/logs.py`).

---

### Task 1: Test scaffold + dev deps + real-session fixture

**Files:**
- Modify: `tools/aletheon-monitor/pyproject.toml`
- Create: `tools/aletheon-monitor/tests/conftest.py`
- Create: `tools/aletheon-monitor/tests/fixtures/real_session_dup.txt`

- [ ] **Step 1: Add pytest as a dev dependency**

Append to `tools/aletheon-monitor/pyproject.toml`:

```toml
[project.optional-dependencies]
dev = [
    "pytest>=8.0",
]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

- [ ] **Step 2: Make `src` importable from tests**

Create `tools/aletheon-monitor/tests/conftest.py`:

```python
"""Ensure the aletheon-monitor package root is importable as `src.*`."""
import os
import sys

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if _ROOT not in sys.path:
    sys.path.insert(0, _ROOT)
```

- [ ] **Step 3: Snapshot the real dup-render session into a repo fixture**

Run (copies the user's real TUI output into the repo so tests are self-contained):

```bash
cd /home/aurobear/Bear-ws/aletheon
mkdir -p tools/aletheon-monitor/tests/fixtures
cp /home/aurobear/Bear-ws/tmp.md tools/aletheon-monitor/tests/fixtures/real_session_dup.txt
wc -l tools/aletheon-monitor/tests/fixtures/real_session_dup.txt
```

Expected: prints a line count around `419 tools/aletheon-monitor/tests/fixtures/real_session_dup.txt`.

- [ ] **Step 4: Verify pytest runs (no tests yet)**

Run:

```bash
cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor
python3 -m pytest -q
```

Expected: `no tests ran` (exit code 5) — confirms pytest is installed and discovery works. If `pytest` is missing, run `python3 -m pip install pytest`.

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/pyproject.toml tools/aletheon-monitor/tests/conftest.py tools/aletheon-monitor/tests/fixtures/real_session_dup.txt
git commit -m "test(monitor): add pytest scaffold + real dup-render fixture"
```

---

### Task 2: `frame.py` — normalize / stability (pure)

**Files:**
- Create: `tools/aletheon-monitor/src/frame.py`
- Test: `tools/aletheon-monitor/tests/test_frame.py`

- [ ] **Step 1: Write the failing tests**

Create `tools/aletheon-monitor/tests/test_frame.py`:

```python
from src.frame import normalize, is_stable, changed


def test_normalize_strips_ansi_and_trailing_blanks():
    raw = "\x1b[1mhello\x1b[0m   \n\nworld  \n\n\n"
    assert normalize(raw) == "hello\n\nworld"


def test_is_stable_true_when_last_three_identical():
    assert is_stable(["a", "b", "x", "x", "x"], window=3) is True


def test_is_stable_false_when_still_changing():
    assert is_stable(["x", "x", "y"], window=3) is False


def test_is_stable_false_when_too_few():
    assert is_stable(["x", "x"], window=3) is False


def test_changed_ignores_ansi_and_trailing_ws():
    assert changed("\x1b[1mhi\x1b[0m  ", "hi") is False
    assert changed("hi", "bye") is True
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_frame.py -q`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.frame'`.

- [ ] **Step 3: Implement `frame.py`**

Create `tools/aletheon-monitor/src/frame.py`:

```python
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_frame.py -q`
Expected: PASS — 5 passed.

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/frame.py tools/aletheon-monitor/tests/test_frame.py
git commit -m "feat(monitor): frame normalize + stability detection"
```

---

### Task 3: `tui_checks.py` — dup-render detector (pure)

**Files:**
- Create: `tools/aletheon-monitor/src/tui_checks.py`
- Test: `tools/aletheon-monitor/tests/test_tui_checks.py`

- [ ] **Step 1: Write the failing test**

Create `tools/aletheon-monitor/tests/test_tui_checks.py`:

```python
from src.tui_checks import check_dup_render


def test_dup_render_detects_repeated_block():
    frame = "\n".join([
        "line one",
        "reflection A",
        "reflection B",
        "reflection C",
        "some other output",
        "reflection A",
        "reflection B",
        "reflection C",
        "tail",
    ])
    findings = check_dup_render(frame, min_block=3)
    assert len(findings) == 1
    assert findings[0]["kind"] == "dup_render"
    assert "reflection A" in findings[0]["evidence"]


def test_dup_render_ignores_short_or_blank_repeats():
    frame = "\n".join(["a", "", "a", "", "b", "", "b"])
    assert check_dup_render(frame, min_block=3) == []
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_checks.py -q`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.tui_checks'`.

- [ ] **Step 3: Implement the dup-render detector**

Create `tools/aletheon-monitor/src/tui_checks.py`:

```python
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_checks.py -q`
Expected: PASS — 2 passed.

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/tui_checks.py tools/aletheon-monitor/tests/test_tui_checks.py
git commit -m "feat(monitor): dup-render detector"
```

---

### Task 4: `tui_checks.py` — remaining checks + `run_checks`, verified on the real fixture

**Files:**
- Modify: `tools/aletheon-monitor/src/tui_checks.py`
- Modify: `tools/aletheon-monitor/tests/test_tui_checks.py`

- [ ] **Step 1: Add failing tests (unit + real-fixture)**

Append to `tools/aletheon-monitor/tests/test_tui_checks.py`:

```python
import os
from src.tui_checks import (
    check_raw_markdown,
    check_double_reflection,
    check_unknown_skill_path,
    check_permission_denied,
    run_checks,
)

_FIXTURE = os.path.join(
    os.path.dirname(__file__), "fixtures", "real_session_dup.txt"
)


def test_raw_markdown_detects_table_pipes():
    assert check_raw_markdown("| # | 问题 |\n|---|------|")[0]["kind"] == "raw_markdown"
    assert check_raw_markdown("normal text") == []


def test_double_reflection_prefix():
    assert check_double_reflection("Reflection: Reflection: 20 calls")[0]["kind"] == "double_reflection"
    assert check_double_reflection("Reflection: 20 calls") == []


def test_unknown_skill_path():
    assert check_unknown_skill_path("未知技能: /home/aurobear/x")[0]["kind"] == "unknown_skill_path"
    assert check_unknown_skill_path("未知技能: foo") == []


def test_permission_denied():
    assert check_permission_denied("touch: Permission denied")[0]["kind"] == "permission_denied"
    assert check_permission_denied("all good") == []


def test_run_checks_on_real_session_fixture():
    """The real captured session must trip the major render/behaviour checks."""
    with open(_FIXTURE, encoding="utf-8", errors="replace") as f:
        frame = f.read()
    kinds = {c["kind"] for c in run_checks(frame)}
    assert "dup_render" in kinds
    assert "raw_markdown" in kinds
    assert "double_reflection" in kinds
    assert "permission_denied" in kinds
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_checks.py -q`
Expected: FAIL — `ImportError: cannot import name 'check_raw_markdown'`.

- [ ] **Step 3: Implement the remaining checks + aggregator**

Append to `tools/aletheon-monitor/src/tui_checks.py`:

```python
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


_CHECKS = [
    check_dup_render,
    check_raw_markdown,
    check_double_reflection,
    check_unknown_skill_path,
    check_permission_denied,
]


def run_checks(frame: str) -> list[dict]:
    """Run every render check over a frame; concatenate findings."""
    findings: list[dict] = []
    for fn in _CHECKS:
        findings.extend(fn(frame))
    return findings
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_checks.py -q`
Expected: PASS — all tests pass, including `test_run_checks_on_real_session_fixture` (proves the checks fire on the real dup-render capture).

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/tui_checks.py tools/aletheon-monitor/tests/test_tui_checks.py
git commit -m "feat(monitor): render checks (markdown/reflection/skill/perm) + real-fixture regression"
```

---

### Task 5: `tui_session.py` — tmux lifecycle + smoke test

**Files:**
- Create: `tools/aletheon-monitor/src/tui_session.py`
- Test: `tools/aletheon-monitor/tests/test_tui_session_smoke.py`

- [ ] **Step 1: Write the smoke test (skips if tmux absent)**

Create `tools/aletheon-monitor/tests/test_tui_session_smoke.py`:

```python
import asyncio
import shutil

import pytest

from src import tui_session as ts

pytestmark = pytest.mark.skipif(
    shutil.which("tmux") is None, reason="tmux not installed"
)


def test_start_send_capture_stop():
    async def scenario():
        s = "aletheon-tui-smoke"
        started = await ts.start("cat", session=s, cols=80, rows=20)
        assert started["ok"], started
        await ts.send("hello-tui", session=s, submit=True)
        await asyncio.sleep(0.3)
        frame = await ts.capture(session=s)
        assert "hello-tui" in frame
        killed = await ts.kill(session=s)
        assert killed["ok"]
        assert await ts.has_session(session=s) is False

    asyncio.run(scenario())
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_session_smoke.py -q`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.tui_session'`.

- [ ] **Step 3: Implement `tui_session.py`**

Create `tools/aletheon-monitor/src/tui_session.py`:

```python
"""tui_session.py — drive and capture a real TUI running inside a tmux pane.

Thin async wrappers over tmux primitives. State (the tmux session) lives in
the OS, so these functions are stateless apart from the session name.
"""
import asyncio

DEFAULT_SESSION = "aletheon-tui-debug"


async def _tmux(*args: str, timeout: float = 5.0) -> tuple[int, str, str]:
    """Run a tmux command, return (returncode, stdout, stderr)."""
    proc = await asyncio.create_subprocess_exec(
        "tmux", *args,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    return (
        proc.returncode,
        out.decode("utf-8", "replace"),
        err.decode("utf-8", "replace"),
    )


async def start(cmd: str, session: str = DEFAULT_SESSION,
                cols: int = 100, rows: int = 40) -> dict:
    """Start `cmd` in a fresh detached tmux session (idempotent)."""
    await kill(session)  # clean slate
    rc, _, err = await _tmux(
        "new-session", "-d", "-s", session,
        "-x", str(cols), "-y", str(rows), cmd,
    )
    if rc != 0:
        return {"ok": False, "error": f"tmux new-session failed: {err.strip()}"}
    return {"ok": True, "session": session, "cols": cols, "rows": rows}


async def send(text: str, session: str = DEFAULT_SESSION,
               submit: bool = True) -> dict:
    """Type literal `text` into the pane; optionally press Enter to submit."""
    rc, _, err = await _tmux("send-keys", "-t", session, "-l", text)
    if rc != 0:
        return {"ok": False, "error": f"send-keys failed: {err.strip()}"}
    if submit:
        rc, _, err = await _tmux("send-keys", "-t", session, "Enter")
        if rc != 0:
            return {"ok": False, "error": f"send Enter failed: {err.strip()}"}
    return {"ok": True}


async def capture(session: str = DEFAULT_SESSION,
                  scrollback: bool = False) -> str:
    """Capture the pane's rendered text. With scrollback, include history."""
    args = ["capture-pane", "-t", session, "-p"]
    if scrollback:
        args += ["-S", "-"]
    rc, out, _ = await _tmux(*args)
    return out if rc == 0 else ""


async def kill(session: str = DEFAULT_SESSION) -> dict:
    """Kill the tmux session if it exists (idempotent)."""
    rc, _, _ = await _tmux("kill-session", "-t", session)
    return {"ok": rc == 0}


async def has_session(session: str = DEFAULT_SESSION) -> bool:
    rc, _, _ = await _tmux("has-session", "-t", session)
    return rc == 0
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_session_smoke.py -q`
Expected: PASS — 1 passed (or skipped if tmux is unavailable; on this box tmux 3.7b is installed, so it should pass).

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/tui_session.py tools/aletheon-monitor/tests/test_tui_session_smoke.py
git commit -m "feat(monitor): tmux TUI session lifecycle (start/send/capture/stop)"
```

---

### Task 6: `tools/tui.py` — MCP wrappers with wait-until-stable capture

**Files:**
- Create: `tools/aletheon-monitor/src/tools/tui.py`
- Test: `tools/aletheon-monitor/tests/test_tui_wrappers.py`

- [ ] **Step 1: Write the failing test (stability loop, tmux-free via monkeypatch)**

Create `tools/aletheon-monitor/tests/test_tui_wrappers.py`:

```python
import asyncio

from src import tui_session as ts
from src.tools import tui as tui_tools


def test_tui_capture_waits_for_stable(monkeypatch):
    frames = iter(["a", "b", "done", "done", "done", "done"])

    async def fake_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        try:
            return next(frames)
        except StopIteration:
            return "done"

    monkeypatch.setattr(ts, "capture", fake_capture)
    res = asyncio.run(tui_tools.tui_capture(
        wait_stable=True, poll=0.01, stable_secs=0.03, timeout=5.0
    ))
    assert res["stable"] is True
    assert res["frame"] == "done"
    assert isinstance(res["checks"], list)


def test_tui_capture_reports_timeout_when_never_stable(monkeypatch):
    counter = {"n": 0}

    async def fake_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        counter["n"] += 1
        return f"frame-{counter['n']}"  # always changing

    monkeypatch.setattr(ts, "capture", fake_capture)
    res = asyncio.run(tui_tools.tui_capture(
        wait_stable=True, poll=0.01, stable_secs=0.03, timeout=0.1
    ))
    assert res["stable"] is False
    assert res["timeout"] is True
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_wrappers.py -q`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.tools.tui'`.

- [ ] **Step 3: Implement `tools/tui.py`**

Create `tools/aletheon-monitor/src/tools/tui.py`:

```python
"""MCP wrappers for driving and capturing the real aletheon TUI via tmux."""
import asyncio
import os
import time

from .. import frame as frame_mod
from .. import tui_checks
from .. import tui_session as ts


def _tui_cmd() -> str:
    """Command that launches the TUI client bound to the daemon socket."""
    override = os.environ.get("ALETHEON_TUI_CMD")
    if override:
        return override
    sock = os.environ.get("ALETHEON_SOCKET", "/run/aletheon/aletheon.sock")
    return f"aletheon --socket {sock}"


async def tui_start(task: str = "", cols: int = 100, rows: int = 40) -> dict:
    """Launch the TUI in tmux; optionally send an initial task. Returns first frame."""
    started = await ts.start(_tui_cmd(), cols=cols, rows=rows)
    if not started.get("ok"):
        return started
    await asyncio.sleep(0.5)  # let the TUI paint its welcome screen
    if task:
        await ts.send(task, submit=True)
    raw = await ts.capture()
    return {"ok": True, "session": started["session"],
            "frame": frame_mod.normalize(raw)}


async def tui_send(text: str, submit: bool = True) -> dict:
    """Type text into the running TUI (optionally submit with Enter)."""
    return await ts.send(text, submit=submit)


async def tui_capture(scrollback: bool = True, wait_stable: bool = True,
                      poll: float = 0.5, stable_secs: float = 1.5,
                      timeout: float = 90.0) -> dict:
    """Capture the TUI frame. With wait_stable, poll until the screen settles
    (no change for `stable_secs`) or `timeout` elapses."""
    if not wait_stable:
        norm = frame_mod.normalize(await ts.capture(scrollback=scrollback))
        return {"stable": None, "frame": norm, "checks": tui_checks.run_checks(norm)}

    window = max(2, int(stable_secs / poll))
    frames: list[str] = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        frames.append(frame_mod.normalize(await ts.capture(scrollback=scrollback)))
        if frame_mod.is_stable(frames, window=window):
            norm = frames[-1]
            return {"stable": True, "frame": norm,
                    "checks": tui_checks.run_checks(norm)}
        await asyncio.sleep(poll)

    norm = frames[-1] if frames else ""
    return {"stable": False, "timeout": True, "frame": norm,
            "checks": tui_checks.run_checks(norm), "last_frames": frames[-3:]}


async def tui_stop() -> dict:
    """Tear down the TUI tmux session."""
    return await ts.kill()
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_tui_wrappers.py -q`
Expected: PASS — 2 passed.

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/tools/tui.py tools/aletheon-monitor/tests/test_tui_wrappers.py
git commit -m "feat(monitor): TUI MCP wrappers with wait-until-stable capture"
```

---

### Task 7: `tools/diagnose.py` — one-stop TUI + daemon bundle with timeline

**Files:**
- Create: `tools/aletheon-monitor/src/tools/diagnose.py`
- Test: `tools/aletheon-monitor/tests/test_diagnose.py`

- [ ] **Step 1: Write the failing test (timeline merge is pure and testable)**

Create `tools/aletheon-monitor/tests/test_diagnose.py`:

```python
from src.tools.diagnose import build_timeline


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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_diagnose.py -q`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.tools.diagnose'`.

- [ ] **Step 3: Implement `tools/diagnose.py`**

Create `tools/aletheon-monitor/src/tools/diagnose.py`:

```python
"""aletheon_diagnose — one-stop bundle: rendered TUI + daemon analyze + logs
+ audit tail, correlated into a single timeline.

Timeline alignment uses the raw ISO timestamp strings each source already
emits (daemon-log granularity); ISO-8601 strings sort chronologically, so no
parsing is required.
"""
import json
import os

from . import analyze as analyze_mod
from . import logs as logs_mod
from . import tui as tui_tools


def _audit_path() -> str:
    return os.environ.get(
        "ALETHEON_AUDIT",
        "/home/aurobear/Bear-ws/aletheon/.aletheon-audit.jsonl",
    )


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


async def diagnose(client, task: str) -> dict:
    """Drive the TUI with `task`, capture the settled frame, and bundle it
    with daemon-side analysis, logs, audit tail, and a merged timeline."""
    started = await tui_tools.tui_start(task=task)
    if not started.get("ok"):
        return {"error": "tui_start failed", "detail": started}

    cap = await tui_tools.tui_capture(scrollback=True, wait_stable=True)
    await tui_tools.tui_stop()

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
    if cap.get("stable") is False:
        verdict = "fail"

    return {
        "task": task,
        "rendered_frame": cap.get("frame", ""),
        "stable": cap.get("stable"),
        "tui_checks": cap.get("checks", []),
        "daemon": {"analyze": daemon_analyze, "logs": daemon_logs},
        "audit_tail": audit_tail,
        "timeline": build_timeline(recent_journal, audit_tail),
        "verdict": verdict,
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest tests/test_diagnose.py -q`
Expected: PASS — 1 passed.

- [ ] **Step 5: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/tools/diagnose.py tools/aletheon-monitor/tests/test_diagnose.py
git commit -m "feat(monitor): aletheon_diagnose bundle + ts-sorted timeline"
```

---

### Task 8: Register the new tools in `server.py`

**Files:**
- Modify: `tools/aletheon-monitor/src/server.py`

- [ ] **Step 1: Add module imports**

In `tools/aletheon-monitor/src/server.py`, extend the `from .tools import (...)` block (currently ends with `watch as watch_mod,`) to add:

```python
    tui as tui_mod,
    diagnose as diagnose_mod,
```

- [ ] **Step 2: Add Tool definitions**

In `server.py`, append these entries to the end of the `TOOLS = [ ... ]` list (after the `aletheon_watch` Tool, before the closing `]`):

```python
    Tool(
        name="aletheon_tui_start",
        description="Launch the real aletheon TUI in a tmux pane (optionally send an initial task). Returns the first rendered frame. Use this to observe what the USER actually sees, not just the RPC response.",
        inputSchema={
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Optional first message to send", "default": ""},
                "cols": {"type": "integer", "description": "Pane width (default 100)", "default": 100},
                "rows": {"type": "integer", "description": "Pane height (default 40)", "default": 40},
            },
        },
    ),
    Tool(
        name="aletheon_tui_send",
        description="Type text into the running TUI pane; submit with Enter by default. Use for multi-turn or slash-command input.",
        inputSchema={
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Literal text to type"},
                "submit": {"type": "boolean", "description": "Press Enter after typing (default true)", "default": True},
            },
            "required": ["text"],
        },
    ),
    Tool(
        name="aletheon_tui_capture",
        description="Capture the current rendered TUI frame. With wait_stable, polls until the screen stops changing (1.5s) or times out (90s). Returns the frame text plus render checks (dup-render, raw markdown, permission-denied, etc).",
        inputSchema={
            "type": "object",
            "properties": {
                "scrollback": {"type": "boolean", "description": "Include scrollback history (default true)", "default": True},
                "wait_stable": {"type": "boolean", "description": "Wait until the frame settles (default true)", "default": True},
            },
        },
    ),
    Tool(
        name="aletheon_tui_stop",
        description="Tear down the TUI tmux session started by aletheon_tui_start.",
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="aletheon_diagnose",
        description="One-stop diagnosis: drives the real TUI with a task, captures the settled frame + render checks, and bundles daemon analyze + logs + audit tail into a single ts-sorted timeline with a pass/fail verdict. Prefer this over aletheon_ask when the bug might be in the TUI layer.",
        inputSchema={
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Task to send to the TUI"},
            },
            "required": ["task"],
        },
    ),
```

- [ ] **Step 3: Add handler dispatch entries**

In `server.py`, add these entries to the `_HANDLERS = { ... }` dict (after the `aletheon_watch` entry, before the closing `}`):

```python
    "aletheon_tui_start": lambda client, args: tui_mod.tui_start(
        task=args.get("task", ""),
        cols=args.get("cols", 100),
        rows=args.get("rows", 40),
    ),
    "aletheon_tui_send": lambda client, args: tui_mod.tui_send(
        text=args.get("text", ""),
        submit=args.get("submit", True),
    ),
    "aletheon_tui_capture": lambda client, args: tui_mod.tui_capture(
        scrollback=args.get("scrollback", True),
        wait_stable=args.get("wait_stable", True),
    ),
    "aletheon_tui_stop": lambda client, args: tui_mod.tui_stop(),
    "aletheon_diagnose": lambda client, args: diagnose_mod.diagnose(
        client, task=args.get("task", ""),
    ),
```

- [ ] **Step 4: Update the module docstring count**

In `server.py`, change the first docstring line `FastMCP server exposing 9 tools` to `FastMCP server exposing 14 tools`.

- [ ] **Step 5: Verify the server imports and lists all 14 tools**

Run:

```bash
cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor
python3 -c "
import sys; sys.path.insert(0, '.')
from src.server import TOOLS, _HANDLERS
names = [t.name for t in TOOLS]
assert len(names) == 14, names
for n in ['aletheon_tui_start','aletheon_tui_send','aletheon_tui_capture','aletheon_tui_stop','aletheon_diagnose']:
    assert n in names and n in _HANDLERS, n
print('OK', len(names), 'tools')
"
```

Expected: `OK 14 tools`.

- [ ] **Step 6: Run the full test suite**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest -q`
Expected: PASS — all tests from Tasks 2–7 pass (tui_session smoke passes with tmux present).

- [ ] **Step 7: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/src/server.py
git commit -m "feat(monitor): register aletheon_tui_* + aletheon_diagnose (14 tools)"
```

---

### Task 9: Add the "TUI track" to the `aletheon-tester` skill

**Files:**
- Modify: `/home/aurobear/Bear-ws/work/aurb/src/skills/general/aletheon-tester/SKILL.md`

> Note: this file is in the **aurb** repo, not aletheon. The design explicitly scopes this edit in. Commit it in the aurb repo separately.

- [ ] **Step 1: Add a "TUI Track" subsection under Phase 2 (Test)**

In `SKILL.md`, immediately after the `### 2.3 Quick Smoke Test` section, insert:

```markdown
### 2.4 TUI Track — test what the USER actually sees

`aletheon_ask` uses `session.ask` RPC and **bypasses the TUI entirely**, so it
cannot see render bugs (duplicate drawing, unrendered markdown, `Reflection:
Reflection:` double-prefix, `未知技能: /path` slash mis-parse). For anything
user-facing, drive the real TUI instead:

```
aletheon_diagnose(task="<the task>")
```

This launches the real `aletheon` TUI in tmux, sends the task, waits for the
frame to settle, and returns:
- `rendered_frame` — what the user actually sees
- `tui_checks` — render assertions that fired (dup_render / raw_markdown /
  double_reflection / unknown_skill_path / permission_denied)
- `daemon.analyze` + `daemon.logs` + `audit_tail` + `timeline`
- `verdict` — pass/fail

Lower-level control if you need it: `aletheon_tui_start` / `aletheon_tui_send`
/ `aletheon_tui_capture` / `aletheon_tui_stop`.
```

- [ ] **Step 2: Add TUI render checks to the Phase 6 acceptance criteria**

In `SKILL.md` under `### 6.2 Acceptance Criteria`, add these bullets to the checklist:

```markdown
- [ ] `aletheon_diagnose` returns `verdict: pass`
- [ ] `tui_checks` is empty (no dup_render / raw_markdown / double_reflection / unknown_skill_path / permission_denied)
- [ ] TUI frame is `stable: true` (no runaway re-render)
```

- [ ] **Step 3: Extend the root-cause table with the TUI layer**

In `SKILL.md` under `### 4.4 Root Cause Classification`, add these rows to the table:

```markdown
| TUI shows duplicated blocks | Double draw: stream append + full-message | `crates/interact/src/tui/response.rs` + `chat.rs` |
| `Reflection: Reflection:` double prefix | Prefix added twice | `crates/interact/src/tui/response.rs:212` |
| `未知技能: /path` on an absolute path | Slash-command parser eats file paths | `crates/interact/src/tui/app/submit.rs:25` |
| Markdown tables printed raw | No table rendering | `crates/interact/src/tui/markdown.rs` |
```

- [ ] **Step 4: Commit (in the aurb repo)**

```bash
cd /home/aurobear/Bear-ws/work/aurb
git add src/skills/general/aletheon-tester/SKILL.md
git commit -m "feat(aletheon-tester): add TUI track — diagnose via real rendered TUI"
```

---

### Task 10: Update monitor README + final suite run

**Files:**
- Modify: `tools/aletheon-monitor/README.md`

- [ ] **Step 1: Document the new tools**

In `tools/aletheon-monitor/README.md`, update the tool count/list to include the 5 new tools. Add this section near the existing tool list:

```markdown
## TUI observability tools

| Tool | Purpose |
|------|---------|
| `aletheon_tui_start` | Launch the real TUI in tmux (optionally send a task); returns first frame |
| `aletheon_tui_send`  | Type text into the running TUI (submit with Enter) |
| `aletheon_tui_capture` | Capture the settled frame + render checks (dup-render, raw markdown, …) |
| `aletheon_tui_stop`  | Tear down the TUI tmux session |
| `aletheon_diagnose`  | One-stop: TUI frame + checks + daemon analyze/logs + audit tail + timeline + verdict |

Requires `tmux`. The TUI command defaults to `aletheon --socket $ALETHEON_SOCKET`;
override with `ALETHEON_TUI_CMD`. Audit path defaults to the repo
`.aletheon-audit.jsonl`; override with `ALETHEON_AUDIT`.
```

- [ ] **Step 2: Final full test run**

Run: `cd /home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor && python3 -m pytest -q`
Expected: PASS — every test green.

- [ ] **Step 3: Commit**

```bash
cd /home/aurobear/Bear-ws/aletheon
git add tools/aletheon-monitor/README.md
git commit -m "docs(monitor): document TUI observability tools"
```

---

## Self-Review

**Spec coverage** (design §5 tool face, §4 units, §9 tests):
- `tui_session.py` → Task 5 · `frame.py` → Task 2 · `tui_checks.py` → Tasks 3–4 · MCP wrappers → Task 6 · `aletheon_diagnose` + timeline → Task 7 · server wiring → Task 8 · skill TUI track → Task 9 · docs → Task 10.
- `tui_checks` fires on the real fixture → Task 4 Step 4 (design §9 acceptance "detect dup-render & raw markdown").
- 1.5s/90s stability defaults → Task 6 `tui_capture` params. Timeline uses raw ISO timestamps (daemon-log granularity) → Task 7 docstring.
- Real fixture = `tmp.md` → Task 1 Step 3.

**Placeholder scan:** none — every step contains runnable code, a concrete command, and an expected result.

**Type/signature consistency:** `tui_session` primitives are `(…, session=DEFAULT_SESSION, …)`; `tui_tools.*` call them without a session arg (single active session). `run_checks(frame) -> list[dict]`; `build_timeline(journal, audit_lines) -> list[dict]`. Handlers return coroutines (all new tool funcs are `async def`), matching `await handler(...)` in `call_tool`.

**Out of scope (unchanged this round):** all T/D/I bug fixes; aletheon Rust source is untouched — only `tools/aletheon-monitor/` and the aurb skill file change.
