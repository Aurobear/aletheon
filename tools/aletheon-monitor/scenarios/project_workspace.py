"""Real project-workspace production scenario with durable boundary evidence."""
from __future__ import annotations
import os
import subprocess
import time
from pathlib import Path
from src import scenarios as base


def _git(root: Path, *args: str) -> str:
    return subprocess.check_output(
        ("git", "-c", f"safe.directory={root}", *args), cwd=root, text=True
    ).strip()


def _scenario_entries(root: Path) -> set[str]:
    scenario_root = root / ".scenario-runs"
    if not scenario_root.exists():
        return set()
    if scenario_root.is_symlink() or not scenario_root.is_dir():
        raise RuntimeError("scenario artifact root is unsafe")
    return {entry.name for entry in scenario_root.iterdir()}


async def run(source_root: str, timeout: float = 120.0) -> dict:
    started = time.monotonic()
    root = Path(source_root).resolve()
    workspace_value = os.environ.get("ALETHEON_PRODUCTION_WORKSPACE", "").strip()
    if not workspace_value:
        return {"scenario": "project_workspace", "status": "BLOCKED",
                "failure": "ALETHEON_PRODUCTION_WORKSPACE is required",
                "assertions": [{"name": "writable_workspace_configured", "passed": False}]}
    workspace = Path(workspace_value)
    if workspace.is_symlink() or not workspace.is_dir():
        return {"scenario": "project_workspace", "status": "FAIL",
                "failure": "production workspace is missing or unsafe", "assertions": []}
    workspace = workspace.resolve()
    if workspace != root:
        return {"scenario": "project_workspace", "status": "FAIL",
                "failure": "source root and production workspace must be the same worktree",
                "assertions": [{"name": "single_project_worktree", "passed": False}]}
    metadata = workspace.stat()
    if metadata.st_uid != os.geteuid() or not os.access(workspace, os.W_OK | os.X_OK):
        return {"scenario": "project_workspace", "status": "FAIL",
                "failure": "production workspace is not owned and writable by the runtime user",
                "assertions": []}
    before = {"root": _git(workspace, "rev-parse", "--show-toplevel"),
              "head": _git(workspace, "rev-parse", "HEAD"),
              "status": _git(workspace, "status", "--porcelain=v1")}
    initial_entries = _scenario_entries(workspace)
    delivery = await base.artifact_delivery(str(workspace), timeout)
    analysis = await base.repository_analysis(str(workspace), timeout)
    boundary = await base.workspace_boundary(str(workspace), timeout)
    after = {"root": _git(workspace, "rev-parse", "--show-toplevel"),
             "head": _git(workspace, "rev-parse", "HEAD"),
             "status": _git(workspace, "status", "--porcelain=v1")}
    final_entries = _scenario_entries(workspace)
    created_entries = final_entries - initial_entries
    assertions = [
        {"name": "known_git_root",
         "passed": Path(before["root"]).resolve() == workspace
         and Path(after["root"]).resolve() == workspace},
        {"name": "initial_worktree_clean", "passed": before["status"] == ""},
        {"name": "git_head_stable", "passed": before["head"] == after["head"]},
        {"name": "worktree_restored", "passed": before["status"] == after["status"]},
        {"name": "scenario_artifacts_scoped",
         "passed": bool(created_entries) and initial_entries <= final_entries},
        {"name": "artifact_delivery", "passed": delivery.get("status") == "PASS"},
        {"name": "repository_analysis", "passed": analysis.get("status") == "PASS"},
        {"name": "outside_write_denied", "passed": boundary.get("status") == "PASS"},
    ]
    return {"scenario": "project_workspace", "status": "PASS" if all(a["passed"] for a in assertions) else "FAIL",
            "assertions": assertions, "duration_ms": round((time.monotonic()-started)*1000),
            "evidence": {"git_before": before, "git_after": after,
                         "workspace": str(workspace),
                         "created_scenario_entries": sorted(created_entries),
                         "cleanup_owner": "aggregate-release-gate",
                         "delivery": delivery, "analysis": analysis, "boundary": boundary}}
