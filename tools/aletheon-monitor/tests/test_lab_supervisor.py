import os
import sys

from src.lab.supervisor import run_bounded


def _run(tmp_path, code, timeout=2.0, capture=1024):
    return run_bounded(
        (sys.executable, "-c", code),
        cwd=tmp_path,
        environment={"PATH": os.environ["PATH"]},
        timeout_seconds=timeout,
        stdout_path=tmp_path / "stdout.log",
        stderr_path=tmp_path / "stderr.log",
        max_capture_bytes=capture,
    )


def test_supervisor_captures_bounded_preview_and_complete_digest(tmp_path):
    result = _run(tmp_path, "print('x' * 4096)", capture=128)
    assert result["exit_code"] == 0
    assert result["timed_out"] is False
    assert result["process_group_reaped"] is True
    assert result["stdout_truncated"] is True
    assert len(result["stdout_preview"].encode()) == 128
    assert result["stdout_digest"].startswith("sha256:")
    assert (tmp_path / "stdout.log").stat().st_size > 4000


def test_supervisor_kills_timed_out_process_group(tmp_path):
    result = _run(
        tmp_path,
        "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)']); time.sleep(30)",
        timeout=0.1,
    )
    assert result["timed_out"] is True
    assert result["process_group_reaped"] is True
    assert result["exit_code"] is not None


def test_supervisor_reports_spawn_failure(tmp_path):
    result = run_bounded(
        (str(tmp_path / "missing-command"),),
        cwd=tmp_path,
        environment={},
        timeout_seconds=1,
        stdout_path=tmp_path / "stdout.log",
        stderr_path=tmp_path / "stderr.log",
        max_capture_bytes=1024,
    )
    assert result["exit_code"] is None
    assert result["spawn_error"].startswith("FileNotFoundError:")
    assert result["process_group_reaped"] is True
