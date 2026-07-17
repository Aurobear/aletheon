import asyncio

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
