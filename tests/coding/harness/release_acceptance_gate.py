#!/usr/bin/env python3
"""Release acceptance gate: verify scoreboard, commit binding, and binary digest.

Called by the release workflow convergence-gate job.  Fails closed.

Performs the authoritative checks required by REL-AUDIT-003:
  1. Invokes verify_run() as the single complete bundle verification.
  2. Requires gate.status == 'passed' with empty reasons.
  3. Requires verified provenance repo.sha == checked-out tag commit.
  4. Downloads and hashes the exact x86_64 ``aletheon`` binary from the
     release tar.gz (without unsafe filesystem extraction) and compares it
     against the verified installed_artifact digest.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
import tarfile
from pathlib import Path

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from acceptance_artifacts import verify_run  # noqa: E402
from acceptance_contract import ContractError  # noqa: E402


def _compute_sha256_from_tar_member(
    tar: tarfile.TarFile, member: tarfile.TarInfo
) -> str:
    """Compute SHA-256 of a regular-file member inside *tar*.

    Reads the member entirely in memory without extracting to the filesystem.
    """
    f = tar.extractfile(member)
    if f is None:
        raise ContractError(f"cannot extract tar member {member.name}")
    try:
        h = hashlib.sha256()
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
        return h.hexdigest()
    finally:
        f.close()


def _find_and_hash_aletheon_binary(archive_path: Path) -> str:
    """Locate the single ``aletheon`` regular-file member in *archive_path*
    and return its SHA-256 hex digest.

    Raises ContractError on:
      - missing, unreadable, or malformed archive
      - zero or more than one member whose name ends with ``/aletheon``
      - a matching member that is not a regular file (symlink, dir, etc.)
    """
    if not archive_path.is_file() or archive_path.is_symlink():
        raise ContractError(f"archive not a regular file: {archive_path}")

    try:
        tf = tarfile.open(archive_path, mode="r:gz")
    except (tarfile.TarError, OSError) as exc:
        raise ContractError(
            f"cannot open archive {archive_path}: {exc}"
        ) from exc

    try:
        try:
            members = tf.getmembers()
        except (tarfile.TarError, OSError) as exc:
            raise ContractError(
                f"error enumerating archive members in {archive_path}: {exc}"
            ) from exc

        aletheon_members = [
            m for m in members
            if m.name.endswith("/aletheon")
        ]

        if len(aletheon_members) == 0:
            raise ContractError(
                f"no member ending with /aletheon"
                f" found in {archive_path}"
            )
        if len(aletheon_members) > 1:
            raise ContractError(
                f"multiple members ending with /aletheon found"
                f" in {archive_path}: "
                + ", ".join(m.name for m in aletheon_members)
            )

        target = aletheon_members[0]
        if not target.isfile():
            raise ContractError(
                f"member ending with /aletheon is not a regular file"
                f" in {archive_path}: {target.name}"
            )

        try:
            return _compute_sha256_from_tar_member(tf, target)
        except (tarfile.TarError, OSError) as exc:
            raise ContractError(
                f"error reading archive member {target.name}: {exc}"
            ) from exc
    finally:
        tf.close()


def run_gate(run_dir: Path, commit: str, archive_path: Path) -> int:
    """Execute the release acceptance gate.  Returns 0 on success, 1 on
    any contract violation.

    This function is the programmatic entry-point; the CLI wrapper
    calls it and maps the return code to ``sys.exit``.
    """
    # 1. Run directory must exist and be a real directory.
    if not run_dir.is_dir() or run_dir.is_symlink():
        print(
            f"ERROR: run directory not found or is a symlink: {run_dir}",
            file=sys.stderr,
        )
        return 1

    # 2. Authoritative bundle verification — every manifest, scoreboard,
    #    provenance, evidence, markdown, and gate-recomputation check.
    try:
        scoreboard = verify_run(run_dir)
    except ContractError as exc:
        print(f"ERROR: acceptance verification failed: {exc}", file=sys.stderr)
        return 1

    # 3. Gate status must be 'passed' with *no* reasons.
    gate = scoreboard["gate"]
    if gate["status"] != "passed":
        print(
            f"ERROR: acceptance gate status is '{gate['status']}',"
            f" expected 'passed'",
            file=sys.stderr,
        )
        reasons = gate.get("reasons") or []
        if reasons:
            print(f"  reasons: {', '.join(reasons)}", file=sys.stderr)
        return 1
    if gate.get("reasons"):
        print(
            "ERROR: acceptance gate reported as passed but has reasons:"
            f" {', '.join(gate['reasons'])}",
            file=sys.stderr,
        )
        return 1

    # 4. Commit binding: the verified provenance repo.sha must equal the
    #    exact checked-out tag commit.
    repo_sha = scoreboard["provenance"]["repo"]["sha"]
    if repo_sha != commit:
        print(
            f"ERROR: acceptance repo SHA {repo_sha}"
            f" does not match checked-out commit {commit}",
            file=sys.stderr,
        )
        return 1
    if scoreboard["provenance"]["repo"]["dirty"]:
        print(
            "ERROR: release acceptance provenance was collected from a dirty tree",
            file=sys.stderr,
        )
        return 1

    # 5. Binary-digest binding: the actual x86_64 ``aletheon`` inside the
    #    release tar.gz must match the verified installed_artifact digest.
    installed_digest = scoreboard["provenance"]["installed_artifact"]["sha256"]
    try:
        binary_digest = _find_and_hash_aletheon_binary(archive_path)
    except ContractError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1

    if binary_digest != installed_digest:
        print(
            f"ERROR: x86_64 binary SHA-256 {binary_digest}"
            f" does not match installed artifact SHA-256 {installed_digest}",
            file=sys.stderr,
        )
        return 1

    print(f"Convergence acceptance gate passed for {run_dir.name}")
    print(f"  commit:        {commit}")
    print(f"  binary digest: {binary_digest}")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Release acceptance gate CLI."
    )
    parser.add_argument(
        "--run-dir", required=True,
        help="Path to the acceptance run directory",
    )
    parser.add_argument(
        "--commit", required=True,
        help="Full commit SHA of the checked-out tag",
    )
    parser.add_argument(
        "--x86-64-archive", required=True,
        help="Path to the x86_64 release .tar.gz archive",
    )
    args = parser.parse_args()
    sys.exit(run_gate(
        run_dir=Path(args.run_dir),
        commit=args.commit,
        archive_path=Path(args.x86_64_archive),
    ))


if __name__ == "__main__":
    main()
