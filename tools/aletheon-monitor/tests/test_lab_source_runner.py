import json
import subprocess
import sys
from pathlib import Path

from src.lab.config import load_settings
from src.lab.model import CaseSpec, DiagnosticSettings, SourceSettings
from src.lab.runner import CaseRunner
from src.lab.source import SourceManager
from src.lab.store import LabStore


def _git(cwd, *arguments):
    return subprocess.run(
        ("git", *arguments),
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def _source_repository(tmp_path):
    source = tmp_path / "upstream"
    source.mkdir()
    _git(source, "init", "-q", "-b", "dev")
    _git(source, "config", "user.name", "Nightwatch Test")
    _git(source, "config", "user.email", "nightwatch@example.invalid")
    (source / "tracked.txt").write_text("baseline\n", encoding="utf-8")
    _git(source, "add", "tracked.txt")
    _git(source, "commit", "-q", "-m", "baseline")
    return source


def test_source_manager_creates_pinned_clean_worktree(tmp_path):
    upstream = _source_repository(tmp_path)
    manager = SourceManager(tmp_path / "state", upstream.as_uri(), "dev")
    commit = manager.sync()
    worktree = manager.create_worktree("run-001", commit)
    assert _git(worktree, "rev-parse", "HEAD") == commit
    assert _git(worktree, "status", "--porcelain") == ""
    manager.remove_worktree(worktree)
    assert not worktree.exists()


def test_runner_seals_pass_and_clusters_repeat_failure(tmp_path):
    upstream = _source_repository(tmp_path)
    manager = SourceManager(tmp_path / "source-state", upstream.as_uri(), "dev")
    commit = manager.sync()
    state = tmp_path / "lab"
    with LabStore(state / "state" / "lab.sqlite") as store:
        runner = CaseRunner(
            state_root=state,
            store=store,
            diagnostics=DiagnosticSettings(enabled=False),
            max_capture_bytes=4096,
        )
        passing_tree = manager.create_worktree("passing-tree", commit)
        try:
            passed = runner.run(
                CaseSpec(
                    case_id="pass.v1",
                    command=(sys.executable, "-c", "print('passed')"),
                ),
                repository_root=passing_tree,
                expected_sha=commit,
                run_id="pass-run",
            )
        finally:
            manager.remove_worktree(passing_tree)
        assert passed["outcome"] == "passed"
        assert Path(passed["bundle_path"], "bundle.json").is_file()

        counts = []
        for suffix in ("one", "two"):
            worktree = manager.create_worktree(f"failure-tree-{suffix}", commit)
            try:
                failed = runner.run(
                    CaseSpec(
                        case_id="failure.v1",
                        command=(
                            sys.executable,
                            "-c",
                            "import sys; print('stable failure', file=sys.stderr); raise SystemExit(7)",
                        ),
                    ),
                    repository_root=worktree,
                    expected_sha=commit,
                    run_id=f"failure-run-{suffix}",
                )
            finally:
                manager.remove_worktree(worktree)
            assert failed["outcome"] == "product_failed"
            assert failed["oracle"]["passed"] is False
            counts.append(failed["cluster"]["occurrence_count"])
        assert counts == [1, 2]
        draft_name = failed["fingerprint"].removeprefix("sha256:") + ".md"
        assert (state / "issue-drafts" / draft_name).is_file()


def test_runner_refuses_dirty_source_without_reporting_pass(tmp_path):
    upstream = _source_repository(tmp_path)
    manager = SourceManager(tmp_path / "source-state", upstream.as_uri(), "dev")
    commit = manager.sync()
    worktree = manager.create_worktree("dirty-tree", commit)
    (worktree / "untracked.txt").write_text("dirty\n")
    state = tmp_path / "lab"
    try:
        with LabStore(state / "state" / "lab.sqlite") as store:
            runner = CaseRunner(
                state_root=state,
                store=store,
                diagnostics=DiagnosticSettings(enabled=False),
                max_capture_bytes=4096,
            )
            result = runner.run(
                CaseSpec(
                    case_id="clean-required.v1",
                    command=(sys.executable, "-c", "raise SystemExit(0)"),
                ),
                repository_root=worktree,
                expected_sha=commit,
                run_id="dirty-run",
            )
    finally:
        manager.remove_worktree(worktree)
    assert result["outcome"] == "invalid"
    assert result["oracle"]["executed"] is False
    assert result["oracle"]["complete"] is False


def test_config_loader_rejects_duplicate_case_ids(tmp_path):
    root = tmp_path / "state"
    config = tmp_path / "nightwatch.toml"
    config.write_text(
        f"""schema_version = 1
[state]
root = {json.dumps(str(root))}
[source]
repository_url = "https://github.com/Aurobear/aletheon.git"
ref = "dev"
[[campaigns]]
case_id = "same.v1"
command = ["true"]
[[campaigns]]
case_id = "same.v1"
command = ["true"]
""",
        encoding="utf-8",
    )
    try:
        load_settings(config)
    except ValueError as error:
        assert "unique" in str(error)
    else:
        raise AssertionError("duplicate case IDs were accepted")


def test_config_loader_requires_explicit_state_root(tmp_path):
    config = tmp_path / "nightwatch.toml"
    config.write_text(
        """schema_version = 1
[state]
[source]
repository_url = "https://github.com/Aurobear/aletheon.git"
[[campaigns]]
case_id = "smoke.v1"
command = ["true"]
""",
        encoding="utf-8",
    )
    try:
        load_settings(config)
    except ValueError as error:
        assert "state.root" in str(error)
    else:
        raise AssertionError("missing state root was accepted")


def test_config_loader_rejects_string_boolean(tmp_path):
    config = tmp_path / "nightwatch.toml"
    config.write_text(
        f"""schema_version = 1
[state]
root = {json.dumps(str(tmp_path / "state"))}
[source]
repository_url = "https://github.com/Aurobear/aletheon.git"
[[campaigns]]
case_id = "smoke.v1"
command = ["true"]
enabled = "false"
""",
        encoding="utf-8",
    )
    try:
        load_settings(config)
    except TypeError as error:
        assert "boolean" in str(error)
    else:
        raise AssertionError("string boolean was accepted")


def test_source_url_rejects_embedded_credentials():
    try:
        SourceSettings("https://token@github.com/Aurobear/aletheon.git")
    except ValueError as error:
        assert "credentials" in str(error)
    else:
        raise AssertionError("credential-bearing source URL was accepted")
