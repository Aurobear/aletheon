"""Pinned source checkout management for continuous campaigns."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


def _git(*argv: str, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        ("git", *argv),
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
    )
    return completed.stdout.strip()


class SourceManager:
    """Maintain a private mirror and detached per-run worktrees."""

    def __init__(self, state_root: Path, repository_url: str, ref: str):
        self.state_root = state_root.resolve()
        self.mirror = self.state_root / "source" / "aletheon.git"
        self.worktrees = self.state_root / "worktrees"
        self.repository_url = repository_url
        self.ref = ref

    def sync(self) -> str:
        self.mirror.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.worktrees.mkdir(parents=True, exist_ok=True, mode=0o700)
        if not self.mirror.exists():
            _git("clone", "--mirror", self.repository_url, str(self.mirror))
        else:
            if (
                _git("--git-dir", str(self.mirror), "remote", "get-url", "origin")
                != self.repository_url
            ):
                raise RuntimeError(
                    "source mirror origin does not match reviewed configuration"
                )
        _git(
            "--git-dir",
            str(self.mirror),
            "fetch",
            "--no-tags",
            "--prune",
            "origin",
            f"+refs/heads/{self.ref}:refs/heads/{self.ref}",
        )
        commit = _git(
            "--git-dir",
            str(self.mirror),
            "rev-parse",
            "--verify",
            f"refs/heads/{self.ref}^{{commit}}",
        )
        if len(commit) != 40 or any(
            character not in "0123456789abcdef" for character in commit
        ):
            raise RuntimeError("source ref did not resolve to a full commit SHA")
        return commit

    def create_worktree(self, run_id: str, commit_sha: str) -> Path:
        target = (self.worktrees / run_id).resolve()
        if target.parent != self.worktrees.resolve():
            raise ValueError("run worktree escaped its managed root")
        if target.exists():
            raise FileExistsError(target)
        _git(
            "--git-dir",
            str(self.mirror),
            "worktree",
            "add",
            "--detach",
            str(target),
            commit_sha,
        )
        head = _git("rev-parse", "HEAD", cwd=target)
        status = _git("status", "--porcelain", cwd=target)
        if head != commit_sha or status:
            self.remove_worktree(target)
            raise RuntimeError("created worktree failed pinned-clean verification")
        return target

    def remove_worktree(self, path: Path) -> None:
        resolved = path.resolve()
        if resolved.parent != self.worktrees.resolve():
            raise ValueError("refusing to remove an unmanaged worktree")
        _git(
            "--git-dir",
            str(self.mirror),
            "worktree",
            "remove",
            "--force",
            str(resolved),
        )
        _git("--git-dir", str(self.mirror), "worktree", "prune")


def repository_facts(root: Path) -> dict[str, object]:
    commit = _git("rev-parse", "HEAD", cwd=root)
    status = _git("status", "--porcelain", "--untracked-files=all", cwd=root)
    return {
        "commit_sha": commit,
        "tree_clean": not bool(status),
        "dirty_paths": status.splitlines()[:100],
    }


def workspace_diff(root: Path, target: Path) -> None:
    descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        completed = subprocess.run(
            ("git", "diff", "--binary", "HEAD", "--"),
            cwd=root,
            check=False,
            stdout=descriptor,
            stderr=subprocess.DEVNULL,
        )
        if completed.returncode != 0:
            raise RuntimeError("failed to capture workspace diff")
    finally:
        os.close(descriptor)
