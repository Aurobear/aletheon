"""Bounded process-group execution owned by Nightwatch."""

from __future__ import annotations

import hashlib
import os
import signal
import subprocess
import time
from collections.abc import Mapping, Sequence
from pathlib import Path


def _digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(block)
    return "sha256:" + value.hexdigest()


def _preview(path: Path, limit: int) -> tuple[str, bool]:
    size = path.stat().st_size
    with path.open("rb") as handle:
        data = handle.read(limit)
    return data.decode("utf-8", errors="replace"), size > len(data)


def _group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _terminate_group(process_group: int) -> bool:
    if not _group_exists(process_group):
        return True
    for requested_signal, grace in ((signal.SIGTERM, 1.0), (signal.SIGKILL, 1.0)):
        try:
            os.killpg(process_group, requested_signal)
        except ProcessLookupError:
            return True
        deadline = time.monotonic() + grace
        while time.monotonic() < deadline:
            if not _group_exists(process_group):
                return True
            time.sleep(0.02)
    return not _group_exists(process_group)


def run_bounded(
    argv: Sequence[str],
    *,
    cwd: Path,
    environment: Mapping[str, str],
    timeout_seconds: float,
    stdout_path: Path,
    stderr_path: Path,
    max_capture_bytes: int,
) -> dict[str, object]:
    """Execute argv without a shell and prove process-group cleanup."""
    if not argv:
        raise ValueError("argv cannot be empty")
    started = time.monotonic()
    spawn_error = None
    timed_out = False
    process: subprocess.Popen[bytes] | None = None
    stdout_descriptor = os.open(
        stdout_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
    )
    stderr_descriptor = os.open(
        stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
    )
    try:
        try:
            process = subprocess.Popen(
                list(argv),
                cwd=cwd,
                env=dict(environment),
                stdin=subprocess.DEVNULL,
                stdout=stdout_descriptor,
                stderr=stderr_descriptor,
                start_new_session=True,
            )
        except OSError as error:
            spawn_error = f"{type(error).__name__}: {error}"
            os.write(stderr_descriptor, spawn_error.encode("utf-8", errors="replace"))
        finally:
            os.close(stdout_descriptor)
            os.close(stderr_descriptor)

        if process is not None:
            try:
                process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired:
                timed_out = True
                _terminate_group(process.pid)
                try:
                    process.wait(timeout=1.0)
                except subprocess.TimeoutExpired:
                    pass
            process_group_reaped = _terminate_group(process.pid)
        else:
            process_group_reaped = True
    except BaseException:
        if process is not None:
            _terminate_group(process.pid)
        raise

    stdout_preview, stdout_truncated = _preview(stdout_path, max_capture_bytes)
    stderr_preview, stderr_truncated = _preview(stderr_path, max_capture_bytes)
    return {
        "argv": list(argv),
        "exit_code": process.returncode if process is not None else None,
        "timed_out": timed_out,
        "spawn_error": spawn_error,
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "process_group_reaped": process_group_reaped,
        "stdout_path": stdout_path.name,
        "stderr_path": stderr_path.name,
        "stdout_digest": _digest(stdout_path),
        "stderr_digest": _digest(stderr_path),
        "stdout_preview": stdout_preview,
        "stderr_preview": stderr_preview,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
    }
