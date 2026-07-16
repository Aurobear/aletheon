"""Real project-workspace production scenario with durable boundary evidence."""
from __future__ import annotations
import subprocess
import time
from pathlib import Path
from src import scenarios as base


def _git(root: Path, *args: str) -> str:
    return subprocess.check_output(("git", *args), cwd=root, text=True).strip()


async def run(source_root: str, timeout: float = 120.0) -> dict:
    started = time.monotonic()
    root = Path(source_root).resolve()
    before = {"root": _git(root, "rev-parse", "--show-toplevel"),
              "head": _git(root, "rev-parse", "HEAD"),
              "status": _git(root, "status", "--porcelain=v1")}
    delivery = await base.artifact_delivery(str(root), timeout)
    analysis = await base.repository_analysis(str(root), timeout)
    boundary = await base.workspace_boundary(str(root), timeout)
    after = {"root": _git(root, "rev-parse", "--show-toplevel"),
             "head": _git(root, "rev-parse", "HEAD"),
             "status": _git(root, "status", "--porcelain=v1")}
    before_lines = set(before["status"].splitlines())
    new_status = set(after["status"].splitlines()) - before_lines
    assertions = [
        {"name": "known_git_root", "passed": Path(before["root"]).resolve() == root},
        {"name": "git_head_stable", "passed": before["head"] == after["head"]},
        {"name": "workspace_status_accounted",
         "passed": all(".scenario-runs/" in line for line in new_status)},
        {"name": "artifact_delivery", "passed": delivery.get("status") == "PASS"},
        {"name": "repository_analysis", "passed": analysis.get("status") == "PASS"},
        {"name": "outside_write_denied", "passed": boundary.get("status") == "PASS"},
    ]
    return {"scenario": "project_workspace", "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
            "assertions": assertions, "duration_ms": round((time.monotonic()-started)*1000),
            "evidence": {"git_before": before, "git_after": after,
                         "delivery": delivery, "analysis": analysis, "boundary": boundary}}
