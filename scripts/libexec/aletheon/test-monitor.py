#!/usr/bin/env python3
"""Run monitor tests with an environment that provides its declared dependencies."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


def usable(candidate: Path) -> bool:
    if not candidate.is_file() or not os.access(candidate, os.X_OK):
        return False
    result = subprocess.run(
        [str(candidate), "-c", "import mcp, pytest"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def select_python() -> Path:
    candidates: list[Path] = []
    configured = os.environ.get("ALETHEON_MONITOR_TEST_PYTHON")
    if configured:
        candidates.append(Path(configured).expanduser())
    candidates.append(Path(sys.executable))
    user_data = Path(
        os.environ.get(
            "ALETHEON_USER_DATA_DIR",
            Path.home() / ".local/share/aletheon",
        )
    )
    candidates.append(user_data / "monitor/.venv/bin/python")
    for candidate in candidates:
        if usable(candidate):
            return candidate
    raise RuntimeError(
        "no Python environment provides monitor dependencies (mcp and pytest); "
        "run the canonical install/deploy or set ALETHEON_MONITOR_TEST_PYTHON"
    )


def main() -> int:
    if len(sys.argv) < 2:
        raise ValueError("at least one monitor test path is required")
    python = select_python()
    command = [str(python), "-m", "pytest", "-q", *sys.argv[1:]]
    return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"aletheon monitor test: {error}", file=sys.stderr)
        raise SystemExit(2)
