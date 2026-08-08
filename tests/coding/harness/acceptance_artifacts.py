"""Acceptance artifacts I/O.

Publishes immutable, verifiable acceptance-run directories with locked,
atomic writes and cross-checked evidence.
"""

from __future__ import annotations

import ctypes
import datetime
import errno
import fcntl
import hashlib
import json
import math
import os
import re
import shutil
import stat
import tempfile
from pathlib import Path, PurePosixPath

from acceptance_contract import (
    ALLOWED_STATUSES,
    ENVIRONMENT_LEVELS,
    ContractError,
    canonical_bytes,
    validate_provenance,
)
from acceptance_scoreboard import (
    build_scoreboard,
    gate_verdict,
    validate_projected_task,
)
from receipt import METRIC_KEYS

_MAX_FILE_SIZE = 16 * 1024 * 1024  # 16 MiB
_MAX_TASKS = 10_000
_MAX_BINARIES = 1_000
_MAX_EVIDENCE_FILES = 1_000
_MAX_CATEGORY_KEYS = 64
_MAX_FAILURE_CLASS_KEYS = 64
_MAX_TERMINAL_KEYS = 64

_RUN_ID_RE = re.compile(r"^[A-Za-z0-9._-]{1,128}$")
_TS_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")

# Linux renameat2 flags
_RENAME_NOREPLACE = 0x1


# ---------------------------------------------------------------------------
#  platform helpers
# ---------------------------------------------------------------------------
def _rename_noreplace(src: str, dst: str) -> None:
    """Atomically rename *src* to *dst*; fail if *dst* already exists."""
    try:
        libc = ctypes.CDLL("libc.so.6", use_errno=True)
    except OSError as exc:
        raise ContractError("cannot load libc for renameat2") from exc
    ret = libc.renameat2(0, src.encode(), 0, dst.encode(), _RENAME_NOREPLACE)
    if ret != 0:
        e = ctypes.get_errno()
        raise OSError(e, os.strerror(e))


def _safe_mkdir(path: Path, parents: bool = False) -> None:
    """Create directory *path* with mode 0o700, then verify it is a real
    directory."""
    try:
        path.mkdir(mode=0o700, parents=parents, exist_ok=False)
    except OSError as exc:
        raise ContractError(f"cannot create directory {path}") from exc
    os.chmod(str(path), 0o700)
    try:
        fd = os.open(str(path), os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as exc:
        raise ContractError(
            f"cannot open freshly created directory {path}"
        ) from exc
    try:
        st = os.fstat(fd)
        if not stat.S_ISDIR(st.st_mode):
            raise ContractError(
                f"created entry is not a directory: {path}"
            )
        if stat.S_IMODE(st.st_mode) != 0o700:
            raise ContractError(
                f"directory mode is not 0700: {path}"
            )
    finally:
        os.close(fd)


def _ensure_evidence_parents(dst_parent: Path, logs_dir: Path) -> None:
    """Create and validate every directory component beneath *logs_dir*
    up to *dst_parent* (exclusive of *logs_dir* itself).

    Each component is created individually with mode 0700 and then
    verified to be a real directory with the correct mode.
    """
    try:
        rel = dst_parent.relative_to(logs_dir)
    except ValueError:
        # Should not happen if dst_parent is under logs_dir
        raise ContractError("evidence destination not inside logs")
    parts = rel.parts
    current = logs_dir
    for part in parts:
        current = current / part
        try:
            fd = os.open(
                str(current),
                os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
            )
        except OSError as exc:
            if exc.errno == errno.ENOENT:
                try:
                    os.mkdir(str(current), 0o700)
                except OSError as exc2:
                    raise ContractError(
                        "cannot create evidence directory"
                    ) from exc2
                os.chmod(str(current), 0o700)
                try:
                    fd = os.open(
                        str(current),
                        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                    )
                except OSError as exc2:
                    raise ContractError(
                        "cannot open newly created evidence directory"
                    ) from exc2
            else:
                raise ContractError(
                    "cannot open evidence directory"
                ) from exc
        try:
            st = os.fstat(fd)
            if not stat.S_ISDIR(st.st_mode):
                raise ContractError(
                    "evidence path is not a directory"
                )
            if stat.S_IMODE(st.st_mode) != 0o700:
                raise ContractError(
                    "evidence directory mode is not 0700"
                )
        finally:
            os.close(fd)


def _write_sync(path: Path, data: bytes) -> None:
    """Write *data* to *path* and fsync."""
    try:
        fd = os.open(
            str(path),
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
        )
    except OSError as exc:
        raise ContractError(f"cannot open {path} for writing") from exc
    try:
        written_total = 0
        while written_total < len(data):
            written = os.write(fd, data[written_total:])
            if written <= 0:
                raise OSError("write error")
            written_total += written
        os.fsync(fd)
    finally:
        os.close(fd)


def _fsync_dir(path: Path) -> None:
    """Best-effort directory fsync."""
    try:
        fd = os.open(str(path), os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    except OSError:
        pass


def _fsync_tree(path: Path) -> None:
    """fsync *path* and every subdirectory inside it."""
    _fsync_dir(path)
    try:
        for entry in os.scandir(path):
            if entry.is_dir(follow_symlinks=False):
                _fsync_tree(Path(entry.path))
    except OSError:
        pass


# ---------------------------------------------------------------------------
#  bounded reads / copying
# ---------------------------------------------------------------------------
def _bounded_read(path: Path) -> bytes:
    """Read entire regular file <= *size_limit* bytes, never following
    symlinks."""
    try:
        fd = os.open(str(path), os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as exc:
        raise ContractError(f"cannot open {path}") from exc
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            raise ContractError(f"{path} is not a regular file")
        if st.st_size > _MAX_FILE_SIZE:
            raise ContractError(f"{path} exceeds size limit")
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(fd, min(65536, _MAX_FILE_SIZE - total + 1))
            if not chunk:
                break
            total += len(chunk)
            if total > _MAX_FILE_SIZE:
                raise ContractError(f"{path} exceeds size limit")
            chunks.append(chunk)
        data = b"".join(chunks)
        if len(data) != st.st_size:
            raise ContractError(f"{path} size mismatch during read")
        return data
    finally:
        os.close(fd)


def _sha256_file(path: Path) -> str:
    """Compute SHA-256 of regular file *path* (no symlinks, size capped)."""
    try:
        fd = os.open(str(path), os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as exc:
        raise ContractError(f"cannot open {path} for hashing") from exc
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            raise ContractError(f"{path} is not a regular file")
        if st.st_size > _MAX_FILE_SIZE:
            raise ContractError(f"{path} exceeds size limit")
        h = hashlib.sha256()
        total = 0
        while True:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            total += len(chunk)
            if total > _MAX_FILE_SIZE:
                raise ContractError(f"{path} exceeds size limit")
            h.update(chunk)
        if total != st.st_size:
            raise ContractError("file size changed during read")
        return h.hexdigest()
    finally:
        os.close(fd)


def _copy_evidence_file(src_fd: int, dst: Path) -> str:
    """Copy already-opened *src_fd* to *dst* atomically and return digest."""
    try:
        st_src = os.fstat(src_fd)
    except OSError as exc:
        raise ContractError("cannot stat evidence source") from exc
    if st_src.st_size > _MAX_FILE_SIZE:
        raise ContractError("evidence source exceeds size limit")

    try:
        dst_fd = os.open(
            str(dst),
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
        )
    except OSError as exc:
        raise ContractError(
            f"cannot create evidence destination {dst}"
        ) from exc
    try:
        h = hashlib.sha256()
        total = 0
        while True:
            chunk = os.read(src_fd, 65536)
            if not chunk:
                break
            total += len(chunk)
            if total > _MAX_FILE_SIZE:
                raise ContractError("evidence source exceeds size limit")
            h.update(chunk)
            write_offset = 0
            while write_offset < len(chunk):
                written = os.write(dst_fd, chunk[write_offset:])
                if written <= 0:
                    raise OSError("write error during evidence copy")
                write_offset += written
        if total != st_src.st_size:
            raise ContractError("evidence file size changed during copy")
        os.fsync(dst_fd)
        return h.hexdigest()
    finally:
        os.close(dst_fd)


def _safe_open_relative(root_fd: int, rel_path: str) -> int:
    """Open *rel_path* relative to *root_fd* without following any symlink.

    Owns a dup of *root_fd* and closes every descriptor it creates.
    Returns exactly one open regular bounded file fd.
    """
    # Validate rel_path
    if os.path.isabs(rel_path):
        raise ContractError("evidence relative path must not be absolute")
    if "\\" in rel_path:
        raise ContractError("evidence relative path contains backslash")
    if any(ord(c) < 32 for c in rel_path):
        raise ContractError("evidence relative path contains control character")
    parts = PurePosixPath(rel_path).parts
    if not parts:
        raise ContractError("evidence relative path is empty")
    for part in parts:
        if part in ("", ".", ".."):
            raise ContractError("evidence relative path has invalid component")
        if "\\" in part:
            raise ContractError(
                "evidence relative path component contains backslash"
            )
        if any(ord(c) < 32 for c in part):
            raise ContractError(
                "evidence relative path component contains control character"
            )

    current_fd = os.dup(root_fd)  # private copy
    try:
        for idx in range(len(parts) - 1):
            part = parts[idx]
            try:
                next_fd = os.open(
                    part,
                    os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                    dir_fd=current_fd,
                )
            except OSError as exc:
                raise ContractError(
                    "cannot traverse evidence directory"
                ) from exc
            # close the previous directory fd; after assignment *current_fd*
            # refers to the just-opened directory, which will be closed in the
            # finally block.
            os.close(current_fd)
            current_fd = next_fd
            try:
                st = os.fstat(current_fd)
            except OSError:
                raise ContractError(
                    "cannot stat evidence directory component"
                )
            if not stat.S_ISDIR(st.st_mode):
                raise ContractError(
                    "evidence path component is not a directory"
                )

        last_part = parts[-1]
        try:
            file_fd = os.open(
                last_part,
                os.O_RDONLY | os.O_NOFOLLOW,
                dir_fd=current_fd,
            )
        except OSError as exc:
            raise ContractError("cannot open evidence file") from exc
        try:
            st = os.fstat(file_fd)
        except OSError:
            os.close(file_fd)
            raise ContractError("cannot stat evidence file")
        if not stat.S_ISREG(st.st_mode):
            os.close(file_fd)
            raise ContractError("evidence file is not a regular file")
        if st.st_size > _MAX_FILE_SIZE:
            os.close(file_fd)
            raise ContractError("evidence file exceeds size limit")
        return file_fd
    finally:
        os.close(current_fd)


# ---------------------------------------------------------------------------
#  public API
# ---------------------------------------------------------------------------
def pretty_json_bytes(data: object) -> bytes:
    """Deterministic, pretty-printed JSON with trailing newline."""
    return (
        json.dumps(
            data, indent=2, sort_keys=True, ensure_ascii=False
        ).encode("utf-8")
        + b"\n"
    )


def scoreboard_file_digest(data: bytes) -> str:
    """Lowercase SHA-256 hex digest."""
    return hashlib.sha256(data).hexdigest()


def render_markdown(scoreboard: dict) -> str:
    """Render a deterministic Markdown summary of *scoreboard*."""
    prov = scoreboard["provenance"]
    gate = scoreboard["gate"]
    installed = prov["installed_artifact"]
    repo = prov["repo"]
    gen_id = prov["generation_id"]
    env = prov["environment_level"]

    def _escape_cell(val: str) -> str:
        return val.replace("|", "&#124;").replace("\n", "\\\\n")

    lines: list[str] = []
    lines.append(f"# Acceptance Run {scoreboard['run_id']}")
    lines.append("")
    lines.append(f"- **Gate status:** {gate['status']}")
    reasons = gate.get("reasons") or []
    reasons_str = ", ".join(reasons) if reasons else "none"
    lines.append(f"- **Gate reasons:** {reasons_str}")
    lines.append(f"- **Environment level:** {env}")
    dirty_marker = " (dirty)" if repo["dirty"] else ""
    lines.append(f"- **Repo SHA:** {repo['sha']}{dirty_marker}")
    lines.append(
        f"- **Installed artifact digest:** {installed['sha256']}"
    )
    lines.append(f"- **Generation ID:** {gen_id}")
    lines.append("")

    header = (
        "| task_id | status | started_at | ended_at | exit_code | "
        "terminal_settlement_count | scope_violation_count | "
        "resource_leak_count | retry_count | evidence_paths |"
    )
    sep = (
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    )
    lines.append(header)
    lines.append(sep)

    for task in scoreboard["tasks"]:
        tid = _escape_cell(task["task_id"])
        status = _escape_cell(task["status"])
        started = _escape_cell(task.get("started_at") or "")
        ended = _escape_cell(task.get("ended_at") or "")
        exit_code = task.get("exit_code")
        exitc = str(exit_code) if exit_code is not None else ""
        settle = str(task.get("terminal_settlement_count", 0))
        scope = str(task.get("scope_violation_count", 0))
        leak = str(task.get("resource_leak_count", 0))
        retries = str(task.get("retry_count", 0))
        evidence = ", ".join(
            _escape_cell(p) for p in (task.get("evidence_paths") or [])
        )
        row = (
            f"| {tid} | {status} | {started} | {ended} | {exitc} | "
            f"{settle} | {scope} | {leak} | {retries} | {evidence} |"
        )
        lines.append(row)

    lines.append("")
    return "\n".join(lines)


def _validate_root(root: Path) -> None:
    if root.is_symlink() or not root.is_dir():
        raise ContractError(f"root is not a real directory: {root}")


def _validate_lock_fd(lock_fd: int) -> None:
    """Raise if *lock_fd* is not a regular file with mode 0600 owned by euid.

    Does **not** close the descriptor.
    """
    try:
        st = os.fstat(lock_fd)
    except OSError as exc:
        raise ContractError("cannot stat lock file") from exc
    if not stat.S_ISREG(st.st_mode):
        raise ContractError("lock file must be a regular file")
    if stat.S_IMODE(st.st_mode) != 0o600:
        raise ContractError("lock file mode must be 0600")
    if st.st_uid != os.geteuid():
        raise ContractError("lock file owned by a different user")


# ---------------------------------------------------------------------------
#  write_run – atomic immutable publish
# ---------------------------------------------------------------------------
def write_run(
    run_id: str,
    report: dict,
    provenance: dict,
    root: Path,
    *,
    evidence_root: str | Path | None = None,
    reference_time: datetime.datetime | None = None,
) -> Path:
    """Atomically create an acceptance run directory under *root*.

    If *evidence_root* is supplied it must be a real directory; every
    log‑relative evidence path declared by the projected tasks is copied
    verbatim into the published run and its SHA‑256 is recorded in the
    manifest.
    """
    if not isinstance(run_id, str) or not _RUN_ID_RE.fullmatch(run_id):
        raise ContractError(
            "run_id must be safe ASCII, nonempty, and at most 128 chars"
        )
    _validate_root(root)

    target = root / run_id
    lock_path = root / f".lock_{run_id}"

    # ------------------------------------------------------------------
    #  cross‑process exclusive lock
    # ------------------------------------------------------------------
    lock_acquired = False
    lock_fd = -1
    try:
        # Try to create a new lock file exclusively.
        try:
            lock_fd = os.open(
                str(lock_path),
                os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
                0o600,
            )
            os.fchmod(lock_fd, 0o600)
            try:
                _validate_lock_fd(lock_fd)
            except ContractError:
                os.close(lock_fd)
                lock_fd = -1
                raise
        except OSError as exc:
            if exc.errno == errno.EEXIST:
                # Open existing and validate ownership/mode
                try:
                    lock_fd = os.open(
                        str(lock_path),
                        os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW,
                    )
                except OSError as exc2:
                    raise ContractError(
                        "cannot open existing lock file"
                    ) from exc2
                try:
                    _validate_lock_fd(lock_fd)
                except ContractError:
                    os.close(lock_fd)
                    lock_fd = -1
                    raise
            else:
                raise ContractError("cannot open lock file") from exc

        # exclusive non‑blocking lock
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(lock_fd)
            lock_fd = -1
            raise ContractError(
                "another writer holds the lock for this run_id"
            )
        lock_acquired = True

        if target.exists() or target.is_symlink():
            raise ContractError("target already exists")

        # ------------------------------------------------------------------
        #  scoreboard & manifest preparation
        # ------------------------------------------------------------------
        scoreboard = build_scoreboard(
            run_id, report, provenance, reference_time=reference_time
        )
        generation_id: str = scoreboard["provenance"]["generation_id"]

        evidence_required = False
        evidence_dict: dict[str, str] = {}
        evidence_paths_set: set[str] = set()
        for task in scoreboard["tasks"]:
            for ep in task.get("evidence_paths", []):
                evidence_paths_set.add(ep)
        if evidence_paths_set:
            evidence_required = True
            if evidence_root is None:
                raise ContractError(
                    "evidence paths present but evidence_root not supplied"
                )
            ev_root = Path(evidence_root)
            if ev_root.is_symlink() or not ev_root.is_dir():
                raise ContractError(
                    "evidence_root must be a real directory"
                )

        # create temporary directory inside root
        tmp_prefix = f".tmp_{run_id}_"
        tmp_dir = Path(
            tempfile.mkdtemp(prefix=tmp_prefix, dir=str(root.resolve()))
        )
        try:
            logs_dir = tmp_dir / "logs"
            _safe_mkdir(logs_dir)

            # copy evidence files with strict containment
            if evidence_required:
                try:
                    ev_root_fd = os.open(
                        str(ev_root),
                        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                    )
                except OSError as exc:
                    raise ContractError(
                        "cannot open evidence_root directory"
                    ) from exc
                try:
                    for epath in sorted(evidence_paths_set):
                        if not epath.startswith("logs/"):
                            raise ContractError(
                                f"evidence path not canonical: {epath}"
                            )
                        rel_part = epath[len("logs/"):]
                        src_fd = _safe_open_relative(ev_root_fd, rel_part)
                        try:
                            dst = logs_dir / rel_part
                            dst_parent = dst.parent
                            _ensure_evidence_parents(dst_parent, logs_dir)
                            digest = _copy_evidence_file(src_fd, dst)
                            evidence_dict[epath] = digest
                        finally:
                            os.close(src_fd)
                finally:
                    os.close(ev_root_fd)

            # produce canonical artifacts
            scoreboard_bytes = pretty_json_bytes(scoreboard)
            markdown_str = render_markdown(scoreboard)
            markdown_bytes = markdown_str.encode("utf-8")

            manifest = {
                "schema_version": 1,
                "run_id": run_id,
                "scoreboard_sha256": scoreboard_file_digest(
                    scoreboard_bytes
                ),
                "markdown_sha256": scoreboard_file_digest(markdown_bytes),
                "evidence_sha256": dict(sorted(evidence_dict.items())),
                "fixture_digest": scoreboard["provenance"]["fixture_digest"],
                "generation_id": generation_id,
                "environment_level": scoreboard["provenance"][
                    "environment_level"
                ],
                "evaluated_at": scoreboard["evaluated_at"],
                "repo_sha": scoreboard["provenance"]["repo"]["sha"],
                "installed_artifact_sha256": scoreboard["provenance"][
                    "installed_artifact"
                ]["sha256"],
                "gate": scoreboard["gate"],
            }
            manifest_bytes = pretty_json_bytes(manifest)

            _write_sync(tmp_dir / "scoreboard.json", scoreboard_bytes)
            _write_sync(tmp_dir / "scoreboard.md", markdown_bytes)
            _write_sync(tmp_dir / "manifest.json", manifest_bytes)

            _fsync_tree(logs_dir)
            _fsync_dir(tmp_dir)

            if target.exists() or target.is_symlink():
                raise ContractError("target already exists")
            _rename_noreplace(str(tmp_dir), str(target))
            _fsync_dir(root)
        except BaseException:
            shutil.rmtree(tmp_dir, ignore_errors=True)
            raise
    finally:
        if lock_acquired:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
        if lock_fd >= 0:
            os.close(lock_fd)

    return target


# ---------------------------------------------------------------------------
#  verify_run – authoritative terminal verification
# ---------------------------------------------------------------------------
def verify_run(path: Path) -> dict:
    """Verify and return a deep copy of the verified scoreboard at *path*.

    Performs all checks described in the audit contract; raises
    ``ContractError`` on any violation.
    """
    if path.is_symlink() or not path.is_dir():
        raise ContractError("run path must be a real directory")

    entries = sorted(os.listdir(path))
    expected_ents = ["logs", "manifest.json", "scoreboard.json", "scoreboard.md"]
    if entries != expected_ents:
        raise ContractError(
            "run directory must contain exactly"
            f" {expected_ents}"
        )

    # --- manifest: read, parse, validate keys/types/formats ---
    manifest_raw = _bounded_read(path / "manifest.json")
    manifest = _parse_json_dict(manifest_raw, "manifest")
    _validate_manifest_structure(manifest, path.name)
    if pretty_json_bytes(manifest) != manifest_raw:
        raise ContractError("manifest.json is not canonical JSON")

    # --- scoreboard: read, parse, validate keys/types, canonical bytes ---
    raw_scoreboard = _bounded_read(path / "scoreboard.json")
    scoreboard = _parse_json_dict(raw_scoreboard, "scoreboard")
    _validate_scoreboard_structure(scoreboard)
    if pretty_json_bytes(scoreboard) != raw_scoreboard:
        raise ContractError("scoreboard.json is not canonical JSON")
    if scoreboard_file_digest(raw_scoreboard) != manifest["scoreboard_sha256"]:
        raise ContractError("scoreboard digest mismatch")

    # --- provenance validation FIRST, before cross-checking with manifest ---
    raw_provenance = scoreboard["provenance"]
    validated_prov = validate_provenance(raw_provenance)
    if canonical_bytes(raw_provenance) != canonical_bytes(validated_prov):
        raise ContractError("scoreboard provenance is not normalized")

    # --- cross‑check manifest against VALIDATED provenance ---
    if manifest["run_id"] != scoreboard["run_id"] or manifest["run_id"] != path.name:
        raise ContractError("run_id mismatch")
    if manifest["fixture_digest"] != validated_prov["fixture_digest"]:
        raise ContractError("fixture_digest mismatch")
    if manifest["generation_id"] != validated_prov["generation_id"]:
        raise ContractError("generation_id mismatch")
    if manifest["environment_level"] != validated_prov["environment_level"]:
        raise ContractError("environment_level mismatch")
    if manifest["repo_sha"] != validated_prov["repo"]["sha"]:
        raise ContractError("repo_sha mismatch")
    if (
        manifest["installed_artifact_sha256"]
        != validated_prov["installed_artifact"]["sha256"]
    ):
        raise ContractError("installed_artifact_sha256 mismatch")
    if manifest["evaluated_at"] != scoreboard["evaluated_at"]:
        raise ContractError("evaluated_at mismatch")
    if manifest["gate"] != scoreboard["gate"]:
        raise ContractError("gate mismatch")

    # --- binaries ---
    binaries = scoreboard["binaries"]
    if not isinstance(binaries, list) or not binaries:
        raise ContractError("binaries must be a nonempty list")
    if len(binaries) > _MAX_BINARIES:
        raise ContractError(f"too many binaries ({len(binaries)})")
    installed_digest = validated_prov["installed_artifact"]["sha256"]
    normalized_installed = _normalize_digest(installed_digest)
    seen_bin_paths: set[str] = set()
    for idx, entry in enumerate(binaries):
        if not isinstance(entry, dict) or set(entry.keys()) != {"path", "sha256"}:
            raise ContractError(
                f"binaries[{idx}] must have exact keys path, sha256"
            )
        bp = entry["path"]
        if not isinstance(bp, str) or not bp:
            raise ContractError(
                f"binaries[{idx}].path must be a nonempty string"
            )
        if bp in seen_bin_paths:
            raise ContractError(f"duplicate binary path")
        seen_bin_paths.add(bp)
        bd = _normalize_digest(entry["sha256"])
        if bd != normalized_installed:
            raise ContractError(
                f"binaries[{idx}] sha256 does not match installed_artifact"
            )
        if len(bp.encode("utf-8")) > 4096:
            raise ContractError("binary path too long")

    # --- tasks ---
    tasks = scoreboard["tasks"]
    if not isinstance(tasks, list):
        raise ContractError("tasks must be a list")
    if len(tasks) > _MAX_TASKS:
        raise ContractError(f"too many tasks ({len(tasks)})")
    task_ids: set[str] = set()
    generation_id = validated_prov["generation_id"]
    evaluated_at = scoreboard["evaluated_at"]
    if not isinstance(evaluated_at, str) or not _TS_RE.fullmatch(evaluated_at):
        raise ContractError("scoreboard evaluated_at missing or malformed")
    try:
        evaluated_at_dt = datetime.datetime.strptime(
            evaluated_at, "%Y-%m-%dT%H:%M:%SZ"
        ).replace(tzinfo=datetime.timezone.utc)
    except ValueError:
        raise ContractError("evaluated_at is not a valid UTC timestamp")
    for task in tasks:
        validate_projected_task(task, generation_id, evaluated_at_dt)
        tid = task["task_id"]
        if tid in task_ids:
            raise ContractError(f"duplicate task_id")
        task_ids.add(tid)
        for ep in task.get("evidence_paths", []):
            if not isinstance(ep, str) or not ep.startswith("logs/"):
                raise ContractError(f"task evidence_path not canonical")

    # verify that manifest evidence keys == union of task evidence_paths
    task_evidence_union = set()
    for t in tasks:
        task_evidence_union.update(t.get("evidence_paths", []))
    if set(manifest["evidence_sha256"].keys()) != task_evidence_union:
        raise ContractError(
            "manifest evidence_sha256 keys do not match task evidence_paths union"
        )

    # gate reconstruction
    recomputed_gate = gate_verdict(tasks)
    if recomputed_gate != scoreboard["gate"]:
        raise ContractError("gate verdict does not match recomputed gate")

    # --- summary ---
    _validate_summary(scoreboard["summary"], tasks)

    # --- metrics ---
    sample_count = sum(1 for t in tasks if t.get("receipt_valid") is True)
    _validate_metrics(scoreboard["metrics"], sample_count)

    # --- markdown ---
    expected_md = render_markdown(scoreboard).encode("utf-8")
    actual_md = _bounded_read(path / "scoreboard.md")
    if expected_md != actual_md:
        raise ContractError("scoreboard.md does not match generated markdown")
    if scoreboard_file_digest(actual_md) != manifest["markdown_sha256"]:
        raise ContractError("markdown digest mismatch")

    # --- evidence integrity ---
    logs_dir = path / "logs"
    if logs_dir.is_symlink() or not logs_dir.is_dir():
        raise ContractError("logs must be a real directory")
    _validate_evidence_files(logs_dir, manifest["evidence_sha256"])

    import copy
    return copy.deepcopy(scoreboard)


# ===========================================================================
# Internal validators
# ===========================================================================

def _normalize_digest(value: str) -> str:
    if not isinstance(value, str):
        raise ContractError("digest must be a string")
    if value.startswith("sha256:"):
        value = value[len("sha256:"):]
    if len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ContractError(
            "digest must be a lowercase hex sha256 string"
        )
    return "sha256:" + value


def _validate_manifest_structure(manifest: dict, run_id_dir: str) -> None:
    """Enforce exact keys and types for manifest before any field is used."""
    required_keys = {
        "schema_version",
        "run_id",
        "scoreboard_sha256",
        "markdown_sha256",
        "evidence_sha256",
        "fixture_digest",
        "generation_id",
        "environment_level",
        "evaluated_at",
        "repo_sha",
        "installed_artifact_sha256",
        "gate",
    }
    if set(manifest.keys()) != required_keys:
        raise ContractError(
            f"manifest must have exact keys {sorted(required_keys)}"
        )

    if (
        not isinstance(manifest["schema_version"], int)
        or manifest["schema_version"] != 1
    ):
        raise ContractError("manifest.schema_version must be integer 1")
    run_id = manifest["run_id"]
    if not isinstance(run_id, str) or not _RUN_ID_RE.fullmatch(run_id):
        raise ContractError("manifest.run_id invalid")
    if run_id != run_id_dir:
        raise ContractError("manifest.run_id != directory name")

    # scoreboard_sha256, markdown_sha256, fixture_digest must be 64-char hex
    for key in (
        "scoreboard_sha256",
        "markdown_sha256",
        "fixture_digest",
        "installed_artifact_sha256",
    ):
        val = manifest[key]
        if (
            not isinstance(val, str)
            or len(val) != 64
            or not all(c in "0123456789abcdef" for c in val)
        ):
            raise ContractError(
                f"manifest.{key} must be a lowercase hex sha256 string"
            )

    # repo_sha accepts 40 or 64-char hex
    repo_sha = manifest["repo_sha"]
    if (
        not isinstance(repo_sha, str)
        or len(repo_sha) not in (40, 64)
        or not all(c in "0123456789abcdef" for c in repo_sha)
    ):
        raise ContractError(
            "manifest.repo_sha must be a lowercase hex string of length 40 or 64"
        )

    if (
        not isinstance(manifest["generation_id"], str)
        or not manifest["generation_id"]
    ):
        raise ContractError(
            "manifest.generation_id must be a nonempty string"
        )
    if manifest["environment_level"] not in ENVIRONMENT_LEVELS:
        raise ContractError("manifest.environment_level invalid")
    if (
        not isinstance(manifest["evaluated_at"], str)
        or not _TS_RE.fullmatch(manifest["evaluated_at"])
    ):
        raise ContractError("manifest.evaluated_at invalid")

    gate = manifest["gate"]
    if (
        not isinstance(gate, dict)
        or set(gate.keys()) != {"status", "reasons"}
    ):
        raise ContractError(
            "manifest.gate must be {status, reasons}"
        )
    if (
        not isinstance(gate["status"], str)
        or gate["status"] not in ("passed", "failed")
    ):
        raise ContractError("manifest.gate.status must be passed or failed")
    if not isinstance(gate["reasons"], list) or not all(
        isinstance(r, str) for r in gate["reasons"]
    ):
        raise ContractError(
            "manifest.gate.reasons must be a list of strings"
        )

    evidence = manifest["evidence_sha256"]
    if not isinstance(evidence, dict):
        raise ContractError("manifest.evidence_sha256 must be a dict")
    if len(evidence) > _MAX_EVIDENCE_FILES:
        raise ContractError(
            f"too many evidence files ({len(evidence)})"
        )
    for k in evidence.keys():
        # key must be a string
        if not isinstance(k, str):
            raise ContractError("evidence key must be a string")
        # encode to verify length and catch encode errors
        try:
            encoded = k.encode("utf-8")
        except UnicodeEncodeError:
            raise ContractError("evidence key must be valid UTF-8")
        if len(encoded) > 4096:
            raise ContractError("evidence key too long")
        # must start with logs/ and have at least one component after it
        if not k.startswith("logs/"):
            raise ContractError("evidence key must start with logs/")
        # no absolute path (reinforces)
        if k.startswith("/"):
            raise ContractError("evidence key must not be absolute")
        # no trailing slash
        if k.endswith("/"):
            raise ContractError("evidence key must not have trailing slash")
        # no backslash and no control or DEL characters
        if "\\" in k:
            raise ContractError("evidence key must not contain backslash")
        if any(ord(c) < 32 or ord(c) == 127 for c in k):
            raise ContractError("evidence key contains invalid character")
        # split raw and validate components
        parts = k.split("/")
        if len(parts) < 2 or parts[0] != "logs":
            raise ContractError("evidence key must be under logs/")
        for part in parts[1:]:
            if part in ("", ".", ".."):
                raise ContractError("evidence key component invalid")
        # digest value: generic 64 lowercase hex
        v = evidence[k]
        if (
            not isinstance(v, str)
            or len(v) != 64
            or not all(c in "0123456789abcdef" for c in v)
        ):
            raise ContractError("evidence digest must be lowercase hex sha256")


def _validate_scoreboard_structure(scoreboard: dict) -> None:
    """Enforce exact keys and basic types for scoreboard."""
    required_keys = {
        "schema_version",
        "run_id",
        "evaluated_at",
        "allowed_statuses",
        "environment_levels",
        "provenance",
        "binaries",
        "tasks",
        "summary",
        "metrics",
        "gate",
    }
    if set(scoreboard.keys()) != required_keys:
        raise ContractError(
            f"scoreboard must have exact keys {sorted(required_keys)}"
        )

    if (
        not isinstance(scoreboard["schema_version"], int)
        or scoreboard["schema_version"] != 1
    ):
        raise ContractError(
            "scoreboard.schema_version must be integer 1"
        )
    if (
        not isinstance(scoreboard["run_id"], str)
        or not _RUN_ID_RE.fullmatch(scoreboard["run_id"])
    ):
        raise ContractError("scoreboard.run_id invalid")
    if (
        not isinstance(scoreboard["evaluated_at"], str)
        or not _TS_RE.fullmatch(scoreboard["evaluated_at"])
    ):
        raise ContractError("scoreboard.evaluated_at invalid")
    if scoreboard["allowed_statuses"] != list(ALLOWED_STATUSES):
        raise ContractError("scoreboard.allowed_statuses mismatch")
    if scoreboard["environment_levels"] != list(ENVIRONMENT_LEVELS):
        raise ContractError("scoreboard.environment_levels mismatch")
    if not isinstance(scoreboard["provenance"], dict):
        raise ContractError("scoreboard.provenance must be a dict")
    if not isinstance(scoreboard["binaries"], list):
        raise ContractError("scoreboard.binaries must be a list")
    if not isinstance(scoreboard["tasks"], list):
        raise ContractError("scoreboard.tasks must be a list")
    if not isinstance(scoreboard["summary"], dict):
        raise ContractError("scoreboard.summary must be a dict")
    if not isinstance(scoreboard["metrics"], dict):
        raise ContractError("scoreboard.metrics must be a dict")
    gate = scoreboard["gate"]
    if (
        not isinstance(gate, dict)
        or set(gate.keys()) != {"status", "reasons"}
    ):
        raise ContractError(
            "scoreboard.gate must be {status, reasons}"
        )
    if (
        not isinstance(gate["status"], str)
        or gate["status"] not in ("passed", "failed")
    ):
        raise ContractError("scoreboard.gate.status must be passed or failed")
    if not isinstance(gate["reasons"], list) or not all(
        isinstance(r, str) for r in gate["reasons"]
    ):
        raise ContractError(
            "scoreboard.gate.reasons must be a list of strings"
        )


def _validate_summary(summary: dict, tasks: list[dict]) -> None:
    _SUMMARY_KEYS = {
        "total",
        "benchmark_outcomes_passed",
        "benchmark_outcomes_failed",
        "completed_engineering_tasks",
        "expected_non_success_outcomes",
        "validation_pass_rate",
        "evidence_complete_success_rate",
        "false_success_count",
        "scope_violation_count",
        "leaked_resource_count",
        "by_category",
        "by_failure_class",
        "by_observed_terminal",
    }
    if set(summary.keys()) != _SUMMARY_KEYS:
        raise ContractError(
            f"summary must have exact keys {sorted(_SUMMARY_KEYS)}"
        )

    # integer counters – basic type checks
    int_keys = {
        "total",
        "benchmark_outcomes_passed",
        "benchmark_outcomes_failed",
        "completed_engineering_tasks",
        "expected_non_success_outcomes",
        "false_success_count",
        "scope_violation_count",
        "leaked_resource_count",
    }
    for k in int_keys:
        v = summary[k]
        if isinstance(v, bool) or not isinstance(v, int) or v < 0:
            raise ContractError(
                f"summary.{k} must be a nonnegative integer"
            )

    # rates – type and range only
    for rate_key in ("validation_pass_rate", "evidence_complete_success_rate"):
        v = summary[rate_key]
        if isinstance(v, bool) or not isinstance(v, (int, float)):
            raise ContractError(
                f"summary.{rate_key} must be a number"
            )
        if math.isnan(v) or math.isinf(v) or not (0.0 <= v <= 1.0):
            raise ContractError(
                f"summary.{rate_key} must be finite in [0,1]"
            )

    total = summary["total"]
    if total != len(tasks):
        raise ContractError("summary.total must equal number of tasks")

    # Reconstruct outcome counts from tasks (outcome_passed bool)
    passed = sum(1 for t in tasks if t.get("outcome_passed") is True)
    failed = total - passed
    if summary["benchmark_outcomes_passed"] != passed:
        raise ContractError("benchmark_outcomes_passed mismatch")
    if summary["benchmark_outcomes_failed"] != failed:
        raise ContractError("benchmark_outcomes_failed mismatch")
    if passed + failed != total:
        raise ContractError("benchmark passed+failed must equal total")
    expected_rate = round(passed / total, 6) if total else 0.0
    if summary["validation_pass_rate"] != expected_rate:
        raise ContractError("validation_pass_rate mismatch")

    # scope and leak exact sums
    scope_sum = sum(t.get("scope_violation_count", 0) for t in tasks)
    leak_sum = sum(t.get("resource_leak_count", 0) for t in tasks)
    if summary["scope_violation_count"] != scope_sum:
        raise ContractError("scope_violation_count mismatch")
    if summary["leaked_resource_count"] != leak_sum:
        raise ContractError("leaked_resource_count mismatch")

    # false_success_count – we only check range, no reconstruction
    if summary["false_success_count"] > total:
        raise ContractError("false_success_count must be <= total")
    if summary["completed_engineering_tasks"] > total:
        raise ContractError(
            "completed_engineering_tasks must be <= total"
        )
    if summary["expected_non_success_outcomes"] > total:
        raise ContractError(
            "expected_non_success_outcomes must be <= total"
        )

    # by_category: reconstruct from tasks
    expected_cat: dict[str, dict[str, int]] = {}
    for task in tasks:
        cat = task.get("category")
        if cat is None:
            continue
        if not isinstance(cat, str):
            raise ContractError("task category must be a string")
        outcome_passed = task.get("outcome_passed", False)
        expected_cat.setdefault(cat, {"total": 0, "passed": 0, "failed": 0})
        expected_cat[cat]["total"] += 1
        if outcome_passed:
            expected_cat[cat]["passed"] += 1
        else:
            expected_cat[cat]["failed"] += 1

    by_cat = summary["by_category"]
    if not isinstance(by_cat, dict):
        raise ContractError("summary.by_category must be a dict")
    if len(by_cat) > _MAX_CATEGORY_KEYS:
        raise ContractError("too many keys in summary.by_category")
    if set(expected_cat.keys()) != set(by_cat.keys()):
        raise ContractError("by_category keys mismatch")
    cat_total_sum = 0
    for cat_key, exp in expected_cat.items():
        actual = by_cat[cat_key]
        if (
            not isinstance(actual, dict)
            or set(actual.keys()) != {"total", "passed", "failed"}
        ):
            raise ContractError(
                f"summary.by_category entry must have total,passed,failed"
            )
        for sub in ("total", "passed", "failed"):
            av = actual[sub]
            if isinstance(av, bool) or not isinstance(av, int) or av < 0:
                raise ContractError(
                    f"summary.by_category value must be nonnegative int"
                )
        if actual["total"] != actual["passed"] + actual["failed"]:
            raise ContractError(
                "summary.by_category total != passed + failed"
            )
        if actual != exp:
            raise ContractError("by_category data mismatch")
        cat_total_sum += actual["total"]
    if cat_total_sum != total:
        raise ContractError(
            "sum of category totals must equal summary.total"
        )

    # --- by_failure_class ---
    mapping_fc = summary["by_failure_class"]
    if not isinstance(mapping_fc, dict):
        raise ContractError("summary.by_failure_class must be a dict")
    if len(mapping_fc) > _MAX_FAILURE_CLASS_KEYS:
        raise ContractError("too many keys in summary.by_failure_class")

    # build expected counts from tasks (only non-zero)
    expected_fc: dict[str, int] = {}
    for task in tasks:
        val = task.get("failure_class")
        if val is None:
            continue
        if not isinstance(val, str):
            raise ContractError("task failure_class must be a string")
        expected_fc[val] = expected_fc.get(val, 0) + 1

    fc_sum = 0
    for k, v in mapping_fc.items():
        if not isinstance(k, str):
            raise ContractError("by_failure_class key must be a string")
        if isinstance(v, bool) or not isinstance(v, int) or v < 0:
            raise ContractError("by_failure_class value must be a nonnegative integer")
        fc_sum += v
        if k in expected_fc:
            if v != expected_fc[k]:
                raise ContractError("by_failure_class count mismatch")
        else:
            if v != 0:
                raise ContractError("by_failure_class key not observed in tasks must be zero")
    if fc_sum != total:
        raise ContractError("by_failure_class sum must equal summary.total")

    # --- by_observed_terminal ---
    mapping_ot = summary["by_observed_terminal"]
    if not isinstance(mapping_ot, dict):
        raise ContractError("summary.by_observed_terminal must be a dict")
    if len(mapping_ot) > _MAX_TERMINAL_KEYS:
        raise ContractError("too many keys in summary.by_observed_terminal")
    ot_sum = 0
    for k, v in mapping_ot.items():
        if not isinstance(k, str):
            raise ContractError("by_observed_terminal key must be a string")
        if isinstance(v, bool) or not isinstance(v, int) or v < 0:
            raise ContractError("by_observed_terminal value must be a nonnegative integer")
        ot_sum += v
    if ot_sum != total:
        raise ContractError("by_observed_terminal sum must equal summary.total")


def _validate_metrics(metrics: dict, sample_count: int) -> None:
    if not isinstance(metrics, dict):
        raise ContractError("metrics must be a dict")
    expected_keys = set(METRIC_KEYS)
    if set(metrics.keys()) != expected_keys:
        raise ContractError(
            f"metrics must have exact keys {sorted(expected_keys)}"
        )

    for mk in expected_keys:
        mv = metrics[mk]
        if not isinstance(mv, dict):
            raise ContractError(f"metrics.{mk} must be a dict")
        required_sub = {"available", "unavailable", "average", "p50", "p95"}
        if set(mv.keys()) != required_sub:
            raise ContractError(
                f"metrics.{mk} must have exact keys"
                f" {sorted(required_sub)}"
            )

        available = mv["available"]
        unavailable = mv["unavailable"]
        if (
            isinstance(available, bool)
            or not isinstance(available, int)
            or available < 0
        ):
            raise ContractError(
                f"metrics.{mk}.available must be a nonnegative integer"
            )
        if (
            isinstance(unavailable, bool)
            or not isinstance(unavailable, int)
            or unavailable < 0
        ):
            raise ContractError(
                f"metrics.{mk}.unavailable must be a nonnegative integer"
            )
        if available + unavailable != sample_count:
            raise ContractError(
                f"metrics.{mk} available+unavailable must equal sample_count"
            )

        if available == 0:
            if (
                mv["average"] is not None
                or mv["p50"] is not None
                or mv["p95"] is not None
            ):
                raise ContractError(
                    f"metrics.{mk} with available=0 must have null stats"
                )
        else:
            if (
                mv["average"] is None
                or mv["p50"] is None
                or mv["p95"] is None
            ):
                raise ContractError(
                    f"metrics.{mk} with available>0 must have non-null stats"
                )

        avg = mv["average"]
        if avg is not None:
            if isinstance(avg, bool) or not isinstance(avg, (int, float)):
                raise ContractError(
                    f"metrics.{mk}.average must be a number or None"
                )
            if math.isnan(avg) or math.isinf(avg) or avg < 0:
                raise ContractError(
                    f"metrics.{mk}.average must be finite non-negative"
                )

        for stat_key in ("p50", "p95"):
            val = mv[stat_key]
            if val is not None:
                if isinstance(val, bool) or not isinstance(val, int):
                    raise ContractError(
                        f"metrics.{mk}.{stat_key} must be an integer or None"
                    )
                if val < 0:
                    raise ContractError(
                        f"metrics.{mk}.{stat_key} must be nonnegative"
                    )


# ---------------------------------------------------------------------------
#  evidence helpers
# ---------------------------------------------------------------------------
def _validate_evidence_files(
    logs_dir: Path, evidence_sha256: dict[str, str]
) -> None:
    if not isinstance(evidence_sha256, dict):
        raise ContractError("manifest.evidence_sha256 must be a dict")

    actual_files: dict[str, str] = {}
    _scan_logs(logs_dir, actual_files)

    expected_paths = set(evidence_sha256.keys())
    actual_paths = set(actual_files.keys())
    if expected_paths != actual_paths:
        raise ContractError("evidence file set mismatch")

    for epath, expected_digest in evidence_sha256.items():
        actual_digest = actual_files[epath]
        if actual_digest != expected_digest:
            raise ContractError("evidence digest mismatch")


def _scan_logs(base: Path, result: dict[str, str]) -> None:
    """Recursively walk logs directory; populate *result* with canonical
    ``logs/...`` paths."""

    def _recurse(current_dir: Path, prefix: str) -> None:
        try:
            entries = list(os.scandir(current_dir))
        except OSError as exc:
            raise ContractError("error scanning logs directory") from exc
        for entry in entries:
            try:
                if entry.is_symlink():
                    raise ContractError("logs contains symlink entry")
                if entry.is_file(follow_symlinks=False):
                    key = prefix + entry.name
                    digest = _sha256_file(Path(entry.path))
                    result[key] = digest
                elif entry.is_dir(follow_symlinks=False):
                    _recurse(
                        Path(entry.path),
                        prefix + entry.name + "/",
                    )
                else:
                    raise ContractError(
                        "logs contains non-regular, non-directory entry"
                    )
            except OSError as exc:
                raise ContractError(
                    "error processing log entry"
                ) from exc

    _recurse(base, "logs/")


def _parse_json_dict(raw: bytes, name: str) -> dict:
    try:
        data = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ContractError(
            f"{name} is not valid JSON or not UTF-8"
        ) from exc
    if not isinstance(data, dict):
        raise ContractError(f"{name} must be a JSON object")
    return data