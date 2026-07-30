#!/usr/bin/env python3
"""Validate repository-local paths cited by top-level engineering docs."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DOCS = (ROOT / "README.md", ROOT / "CONTRIBUTING.md", ROOT / "SECURITY.md")
REPOSITORY_PREFIXES = ("crates/", "examples/", "docs/", "scripts/", "tests/", "config/")


def local_targets(line: str) -> list[str]:
    targets = re.findall(r"`([^`]+)`", line)
    targets.extend(re.findall(r"\[[^]]*\]\(([^)#]+)", line))
    return targets


def check() -> list[str]:
    failures: list[str] = []
    for document in DOCS:
        for line_number, line in enumerate(document.read_text(encoding="utf-8").splitlines(), 1):
            for target in local_targets(line):
                target = target.strip()
                if "://" in target or not target.startswith(REPOSITORY_PREFIXES):
                    continue
                if any(character in target for character in " <>|*{}"):
                    continue
                if not (ROOT / target).exists():
                    failures.append(
                        f"{document.relative_to(ROOT)}:{line_number}: missing repository path `{target}`"
                    )
    return failures


def main() -> int:
    failures = check()
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print("documentation paths: pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
