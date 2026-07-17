import asyncio
import subprocess

from scenarios import project_workspace


def test_project_scenario_blocks_without_explicit_workspace(monkeypatch, tmp_path):
    monkeypatch.delenv("ALETHEON_PRODUCTION_WORKSPACE", raising=False)
    result = asyncio.run(project_workspace.run(str(tmp_path)))
    assert result["status"] == "BLOCKED"
    assert result["assertions"] == [
        {"name": "writable_workspace_configured", "passed": False}
    ]


def test_project_scenario_rejects_symlink_workspace(monkeypatch, tmp_path):
    target = tmp_path / "target"
    target.mkdir()
    link = tmp_path / "workspace"
    link.symlink_to(target, target_is_directory=True)
    monkeypatch.setenv("ALETHEON_PRODUCTION_WORKSPACE", str(link))
    result = asyncio.run(project_workspace.run(str(tmp_path)))
    assert result["status"] == "FAIL"
    assert "unsafe" in result["failure"]


def test_project_scenario_rejects_split_source_and_workspace(monkeypatch, tmp_path):
    source = tmp_path / "source"
    workspace = tmp_path / "workspace"
    source.mkdir()
    workspace.mkdir()
    monkeypatch.setenv("ALETHEON_PRODUCTION_WORKSPACE", str(workspace))

    result = asyncio.run(project_workspace.run(str(source)))

    assert result["status"] == "FAIL"
    assert result["assertions"] == [
        {"name": "single_project_worktree", "passed": False}
    ]


def test_project_scenario_uses_single_git_worktree_and_retains_evidence(monkeypatch, tmp_path):
    workspace = tmp_path / "candidate-worktree"
    workspace.mkdir()
    subprocess.run(("git", "init", "-q"), cwd=workspace, check=True)
    subprocess.run(("git", "config", "user.name", "Scenario Test"), cwd=workspace, check=True)
    subprocess.run(("git", "config", "user.email", "scenario@example.invalid"),
                   cwd=workspace, check=True)
    (workspace / "tracked.txt").write_text("candidate\n", encoding="utf-8")
    subprocess.run(("git", "add", "tracked.txt"), cwd=workspace, check=True)
    subprocess.run(("git", "commit", "-q", "-m", "candidate"), cwd=workspace, check=True)
    initial = workspace / ".scenario-runs" / "preexisting"
    initial.mkdir(parents=True)
    (workspace / ".gitignore").write_text(".scenario-runs/\n", encoding="utf-8")
    subprocess.run(("git", "add", ".gitignore"), cwd=workspace, check=True)
    subprocess.run(("git", "commit", "-q", "-m", "ignore scenario evidence"),
                   cwd=workspace, check=True)
    calls = []

    async def scenario_stub(source_root, _timeout):
        scenario_root = workspace / ".scenario-runs" / f"generated-{len(calls)}"
        scenario_root.mkdir(parents=True)
        (scenario_root / "artifact.txt").write_text("evidence\n", encoding="utf-8")
        calls.append(source_root)
        return {"status": "PASS"}

    monkeypatch.setattr(project_workspace.base, "artifact_delivery", scenario_stub)
    monkeypatch.setattr(project_workspace.base, "repository_analysis", scenario_stub)
    monkeypatch.setattr(project_workspace.base, "workspace_boundary", scenario_stub)
    monkeypatch.setenv("ALETHEON_PRODUCTION_WORKSPACE", str(workspace))

    result = asyncio.run(project_workspace.run(str(workspace)))

    assert result["status"] == "PASS"
    assert calls == [str(workspace.resolve())] * 3
    assert sorted(path.name for path in (workspace / ".scenario-runs").iterdir()) == [
        "generated-0", "generated-1", "generated-2", "preexisting"
    ]
    assert result["evidence"]["git_before"] == result["evidence"]["git_after"]
    assert result["evidence"]["created_scenario_entries"] == [
        "generated-0", "generated-1", "generated-2"
    ]
    assert result["evidence"]["cleanup_owner"] == "aggregate-release-gate"
