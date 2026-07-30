#!/usr/bin/env python3
"""Validate repository-local paths cited by maintained engineering docs."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
TOP_LEVEL_DOCS = (ROOT / "README.md", ROOT / "CONTRIBUTING.md", ROOT / "SECURITY.md")
DOC_TREES = (
    ROOT / "docs/design",
    ROOT / "docs/deployment",
    ROOT / "docs/guide",
    ROOT / "docs/testing",
    ROOT / "deploy",
    ROOT / "examples",
)
REPOSITORY_PREFIXES = (
    "crates/",
    "deploy/",
    "examples/",
    "docs/",
    "scripts/",
    "tests/",
    "config/",
)


def documents() -> list[Path]:
    discovered = set(TOP_LEVEL_DOCS)
    discovered.update((ROOT / "crates").glob("*/README.md"))
    for tree in DOC_TREES:
        discovered.update(tree.rglob("*.md"))
    return sorted(discovered)


def normalize_target(target: str) -> str:
    """Remove source locators while preserving ordinary path punctuation."""
    return re.sub(r":\d+(?:-\d+)?$", "", target.strip())


def repository_path(document: Path, target: str, is_link: bool) -> Path | None:
    if target.startswith(REPOSITORY_PREFIXES):
        return ROOT / target
    if is_link or target.startswith(("./", "../")):
        return document.parent / target
    if target.endswith(".md"):
        return ROOT / target
    first, separator, remainder = target.partition("/")
    if separator and (ROOT / "crates" / first / "Cargo.toml").is_file():
        return ROOT / "crates" / first / remainder
    return None


def local_targets(line: str) -> list[tuple[str, bool]]:
    targets = [(target, False) for target in re.findall(r"`([^`]+)`", line)]
    targets.extend(
        (target, True) for target in re.findall(r"\[[^]]*\]\(([^)#]+)", line)
    )
    return targets


def check() -> list[str]:
    failures: list[str] = []
    for document in documents():
        content = document.read_text(encoding="utf-8")
        # Historical target documents deliberately retain removed locators as
        # design evidence. They must identify themselves explicitly instead of
        # being mistaken for a current implementation map.
        if "> **Status:** Historical" in content:
            continue
        for line_number, line in enumerate(content.splitlines(), 1):
            for target, is_link in local_targets(line):
                target = normalize_target(target)
                if "://" in target:
                    continue
                if any(character in target for character in " <>|*{}"):
                    continue
                path = repository_path(document, target, is_link)
                if path is not None and not path.exists():
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
