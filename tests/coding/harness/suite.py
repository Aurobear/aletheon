#!/usr/bin/env python3
"""Sequential coding benchmark execution and deterministic suite aggregation."""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import math
import os
import pathlib
import stat
import subprocess
from collections import Counter
from collections.abc import Mapping, Sequence
from copy import deepcopy
from datetime import datetime, timezone

from acceptance_artifacts import write_run
from acceptance_contract import ContractError as AcceptanceContractError
from acceptance_provenance import ProvenanceCollectionError, collect_installed_provenance
from contracts import BenchmarkTask, ContractError, load_catalog
from receipt import FAILURE_CLASSES, METRIC_KEYS, digest, seal, verify_receipt
from run import ROOT, run_task

REPORT_SCHEMA_VERSION = 1
ACCEPTANCE_INPUT_DIGEST_VERSION = b"aletheon-acceptance-inputs-v1\0"
MAX_ACCEPTANCE_INPUT_ENTRIES = 4_096
MAX_ACCEPTANCE_INPUT_FILE_BYTES = 4 * 1024 * 1024
MAX_ACCEPTANCE_INPUT_TOTAL_BYTES = 64 * 1024 * 1024
MAX_ACCEPTANCE_INPUT_PATH_BYTES = 4_096
ACCEPTANCE_INPUT_READ_CHUNK = 64 * 1024

# The only execution statuses allowed by the acceptance scoreboard
# (A1-AUDIT-004 / plan spec:636-648). A `waived` entry additionally requires
# approver/reason/expiry and must not be used to excuse a P0 failure.
EXECUTION_STATUSES = ("not_run", "infra_blocked", "failed", "passed", "waived")


def utc_now() -> str:
    """Return current UTC time as ISO 8601 second-precision with Z suffix."""
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _checked_repository_path(
    root: pathlib.Path,
    path: pathlib.Path,
    *,
    expected: str,
) -> tuple[pathlib.Path, str]:
    """Return a repository-contained path after rejecting every symlink component."""
    try:
        repository = root.resolve(strict=True)
    except OSError as error:
        raise ContractError("acceptance repository root is unavailable") from error
    candidate = path if path.is_absolute() else repository / path
    candidate = pathlib.Path(os.path.abspath(candidate))
    try:
        relative = candidate.relative_to(repository)
    except ValueError as error:
        raise ContractError("acceptance input escapes repository root") from error

    current = repository
    try:
        for part in relative.parts:
            current = current / part
            mode = current.lstat().st_mode
            if stat.S_ISLNK(mode):
                raise ContractError("acceptance input contains a symlink")
    except OSError as error:
        raise ContractError("acceptance input is unavailable") from error

    try:
        resolved = current.resolve(strict=True)
        resolved.relative_to(repository)
    except (OSError, ValueError) as error:
        raise ContractError("acceptance input escapes repository root") from error
    mode = current.stat().st_mode
    if expected == "file" and not stat.S_ISREG(mode):
        raise ContractError("acceptance input must be a regular file")
    if expected == "directory" and not stat.S_ISDIR(mode):
        raise ContractError("acceptance input must be a directory")
    return current, relative.as_posix()


class _AcceptanceInputBudget:
    """Bound the amount of repository input inspected for one digest."""

    def __init__(self) -> None:
        self.entries = 0
        self.total_bytes = 0

    def reserve(self, body_bytes: int, *, regular_file: bool) -> None:
        self.entries += 1
        if self.entries > MAX_ACCEPTANCE_INPUT_ENTRIES:
            raise ContractError("acceptance input contains too many entries")
        if regular_file and body_bytes > MAX_ACCEPTANCE_INPUT_FILE_BYTES:
            raise ContractError("acceptance input file exceeds size limit")
        self.total_bytes += body_bytes
        if self.total_bytes > MAX_ACCEPTANCE_INPUT_TOTAL_BYTES:
            raise ContractError("acceptance input exceeds total size limit")


def _hash_input_header(
    hasher, kind: bytes, relative: str, body_bytes: int
) -> None:
    """Hash a typed, length-delimited acceptance input header."""
    encoded = relative.encode("utf-8")
    if len(encoded) > MAX_ACCEPTANCE_INPUT_PATH_BYTES:
        raise ContractError("acceptance input path exceeds size limit")
    hasher.update(len(kind).to_bytes(8, "big"))
    hasher.update(kind)
    hasher.update(len(encoded).to_bytes(8, "big"))
    hasher.update(encoded)
    hasher.update(body_bytes.to_bytes(8, "big"))


def _stat_identity(value: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        value.st_dev,
        value.st_ino,
        value.st_mode,
        value.st_size,
        value.st_mtime_ns,
        value.st_ctime_ns,
    )


def _hash_regular_file(
    hasher,
    budget: _AcceptanceInputBudget,
    root: pathlib.Path,
    path: pathlib.Path,
    *,
    kind: bytes,
) -> None:
    """Hash one stable regular file through a no-follow descriptor."""
    checked, relative = _checked_repository_path(root, path, expected="file")
    try:
        path_before = checked.lstat()
        flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0)
        flags |= getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(checked, flags)
    except OSError as error:
        raise ContractError("acceptance input file is unreadable") from error
    try:
        descriptor_before = os.fstat(descriptor)
        if not stat.S_ISREG(descriptor_before.st_mode):
            raise ContractError("acceptance input must be a regular file")
        if _stat_identity(path_before) != _stat_identity(descriptor_before):
            raise ContractError("acceptance input file changed before read")
        budget.reserve(descriptor_before.st_size, regular_file=True)
        _hash_input_header(hasher, kind, relative, descriptor_before.st_size)
        total = 0
        while True:
            try:
                chunk = os.read(descriptor, ACCEPTANCE_INPUT_READ_CHUNK)
            except OSError as error:
                raise ContractError("acceptance input file is unreadable") from error
            if not chunk:
                break
            total += len(chunk)
            if total > descriptor_before.st_size:
                raise ContractError("acceptance input file changed during read")
            hasher.update(chunk)
        descriptor_after = os.fstat(descriptor)
        if total != descriptor_before.st_size or (
            _stat_identity(descriptor_before) != _stat_identity(descriptor_after)
        ):
            raise ContractError("acceptance input file changed during read")
        try:
            path_after = checked.lstat()
        except OSError as error:
            raise ContractError("acceptance input file changed during read") from error
        if _stat_identity(path_before) != _stat_identity(path_after):
            raise ContractError("acceptance input file changed during read")
    finally:
        os.close(descriptor)


def _hash_regular_tree(
    hasher,
    budget: _AcceptanceInputBudget,
    root: pathlib.Path,
    directory: pathlib.Path,
) -> None:
    """Hash a complete regular-file tree without following special entries."""
    checked, relative = _checked_repository_path(root, directory, expected="directory")
    budget.reserve(0, regular_file=False)
    _hash_input_header(hasher, b"directory", relative, 0)

    def visit(parent: pathlib.Path) -> None:
        children: list[pathlib.Path] = []
        try:
            with os.scandir(parent) as iterator:
                for entry in iterator:
                    children.append(pathlib.Path(entry.path))
                    if (
                        len(children) + budget.entries
                        > MAX_ACCEPTANCE_INPUT_ENTRIES
                    ):
                        raise ContractError(
                            "acceptance input contains too many entries"
                        )
        except OSError as error:
            raise ContractError("acceptance input tree is unreadable") from error
        for child in sorted(children, key=lambda item: item.name):
            try:
                mode = child.lstat().st_mode
            except OSError as error:
                raise ContractError("acceptance input tree is unreadable") from error
            if stat.S_ISLNK(mode):
                raise ContractError("acceptance input tree contains a symlink")
            _, child_relative = _checked_repository_path(
                root,
                child,
                expected="directory" if stat.S_ISDIR(mode) else "file",
            )
            if stat.S_ISDIR(mode):
                budget.reserve(0, regular_file=False)
                _hash_input_header(hasher, b"directory", child_relative, 0)
                visit(child)
            elif stat.S_ISREG(mode):
                _hash_regular_file(
                    hasher,
                    budget,
                    root,
                    child,
                    kind=b"file",
                )
            else:
                raise ContractError("acceptance input tree contains a non-regular entry")

    visit(checked)


def _catalog_task_paths(
    root: pathlib.Path, catalog: pathlib.Path
) -> list[pathlib.Path]:
    """Enumerate a bounded, real catalog without resolving symlink aliases."""
    checked, _ = _checked_repository_path(root, catalog, expected="directory")
    paths: list[pathlib.Path] = []
    scanned = 0
    task_bytes = 0
    try:
        with os.scandir(checked) as iterator:
            for entry in iterator:
                scanned += 1
                if scanned > MAX_ACCEPTANCE_INPUT_ENTRIES:
                    raise ContractError(
                        "acceptance catalog contains too many entries"
                    )
                if entry.name.endswith(".toml"):
                    path, _ = _checked_repository_path(
                        root, pathlib.Path(entry.path), expected="file"
                    )
                    size = path.lstat().st_size
                    if size > MAX_ACCEPTANCE_INPUT_FILE_BYTES:
                        raise ContractError(
                            "acceptance task definition exceeds size limit"
                        )
                    task_bytes += size
                    if task_bytes > MAX_ACCEPTANCE_INPUT_TOTAL_BYTES:
                        raise ContractError(
                            "acceptance task definitions exceed total size limit"
                        )
                    paths.append(path)
    except OSError as error:
        raise ContractError("acceptance catalog is unreadable") from error
    return sorted(paths, key=lambda item: item.name)


def acceptance_input_digest(
    tasks: Sequence[BenchmarkTask], root: pathlib.Path
) -> str:
    """Bind task definitions, fixture trees, and hidden acceptance trees."""
    hasher = hashlib.sha256()
    hasher.update(ACCEPTANCE_INPUT_DIGEST_VERSION)
    budget = _AcceptanceInputBudget()
    for task in sorted(tasks, key=lambda item: item.id):
        _hash_regular_file(
            hasher,
            budget,
            root,
            task.source,
            kind=b"task",
        )
        _hash_regular_tree(
            hasher,
            budget,
            root,
            root / "tests/coding/fixtures" / task.fixture,
        )
        _hash_regular_tree(
            hasher,
            budget,
            root,
            root / "tests/coding/acceptance" / task.id,
        )
    return "sha256:" + hasher.hexdigest()


def _git_acceptance_state(root: pathlib.Path) -> dict[str, object]:
    """Capture immutable Git identity plus a digest of the exact dirty state."""
    repository = root.resolve(strict=True)

    def run(*arguments: str) -> bytes:
        try:
            result = subprocess.run(
                ["git", "-C", str(repository), *arguments],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                check=False,
            )
        except OSError as error:
            raise ContractError("acceptance repository state is unavailable") from error
        if result.returncode != 0:
            raise ContractError("acceptance repository state is unavailable")
        return result.stdout

    try:
        top = pathlib.Path(run("rev-parse", "--show-toplevel").decode("utf-8").strip())
        sha = run("rev-parse", "HEAD").decode("ascii").strip()
    except (UnicodeError, ValueError) as error:
        raise ContractError("acceptance repository identity is malformed") from error
    if top.resolve(strict=True) != repository:
        raise ContractError("acceptance root is not the repository root")
    if len(sha) not in {40, 64} or any(
        character not in "0123456789abcdef" for character in sha
    ):
        raise ContractError("acceptance repository SHA is malformed")
    status = run(
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignore-submodules=none",
    )
    return {
        "repo_sha": sha,
        "repo_dirty": bool(status),
        "repo_status_digest": digest(status),
    }


def capture_acceptance_source_state(
    root: pathlib.Path, catalog: pathlib.Path
) -> dict[str, object]:
    """Capture the source identity whose installed acceptance run is authoritative."""
    tasks = load_catalog(_catalog_task_paths(root, catalog), root)
    if not tasks:
        raise ContractError("catalog is empty")
    return {
        **_git_acceptance_state(root),
        "input_digest": acceptance_input_digest(tasks, root),
    }


def assert_acceptance_source_unchanged(
    before: Mapping[str, object], after: Mapping[str, object]
) -> None:
    """Fail closed rather than publish evidence spanning mixed source states."""
    required = {
        "repo_sha",
        "repo_dirty",
        "repo_status_digest",
        "input_digest",
    }
    if set(before) != required or set(after) != required or dict(before) != dict(after):
        raise ContractError("acceptance source changed during execution")


def assert_acceptance_provenance_matches_source(
    source: Mapping[str, object], provenance: Mapping[str, object]
) -> None:
    """Require collected provenance to describe the executed source snapshot."""
    repo = provenance.get("repo")
    fixture_digest = provenance.get("fixture_digest")
    if (
        not isinstance(repo, Mapping)
        or repo.get("sha") != source.get("repo_sha")
        or repo.get("dirty") is not source.get("repo_dirty")
        or not isinstance(fixture_digest, str)
        or "sha256:" + fixture_digest != source.get("input_digest")
    ):
        raise ContractError("acceptance provenance does not match executed source")


def _entry_status(entry: Mapping[str, object]) -> str:
    """Map an aggregated task entry to the canonical scoreboard status."""
    if entry.get("receipt_valid") is False and entry.get("receipt_path") is not None:
        return "failed"
    if entry.get("failure_class") == "infrastructure_failure":
        return "infra_blocked"
    if bool(entry.get("outcome_passed")):
        return "passed"
    return "failed"


def _is_p0_failure(entry: Mapping[str, object]) -> bool:
    """A P0 semantic/safety failure must fail the whole release gate."""
    if bool(entry.get("outcome_passed")):
        return False
    reasons = entry.get("reasons") or []
    class_ = str(entry.get("failure_class", ""))
    return (
        bool(entry.get("scope_violation"))
        or bool(entry.get("resource_leak"))
        or bool(entry.get("false_success"))
        or "permission" in " ".join(str(r) for r in reasons).lower()
        or class_ in {"policy_denial", "scope_violation", "unsafe_success"}
    )


def _acceptance_task(
    entry: Mapping[str, object],
    receipt: dict | None,
    receipt_path: str | None,
    started_at: str | None,
    ended_at: str | None,
    generation_id: str,
) -> dict[str, object]:
    """Project an aggregated entry and optional receipt onto the acceptance scoreboard entry keys."""
    outcome_passed = bool(entry.get("outcome_passed", False))
    failure_class = str(entry.get("failure_class", ""))
    reasons = list(entry.get("reasons", []) or [])
    scope_violation_count = 1 if bool(entry.get("scope_violation", False)) else 0
    resource_leak_count = 1 if bool(entry.get("resource_leak", False)) else 0
    receipt_valid = bool(entry.get("receipt_valid", False))

    if not receipt_valid or receipt is None:
        execution_present = isinstance(receipt_path, str)
        receipt_valid = False
        outcome_passed = False
        terminal_settlement_count = 0
        retry_count = 0
        exit_code = None
        expected_terminal = entry.get("expected_terminal")
        observed_terminal = None
        if execution_present:
            evidence_paths = [receipt_path]
        else:
            started_at = None
            ended_at = None
            evidence_paths = []
    else:
        execution_present = True
        evidence_list = list(receipt.get("evidence", []) or [])
        terminal_settlement_count = sum(
            1
            for e in evidence_list
            if isinstance(e, dict) and e.get("kind") == "terminal_snapshot"
        )
        metrics = receipt.get("metrics", {}) or {}
        retry_val = metrics.get("provider_retries")
        if retry_val is None or isinstance(retry_val, bool) or not isinstance(retry_val, int) or retry_val < 0:
            outcome_passed = False
            reasons = reasons + ["retry_evidence_missing"]
            retry_count = 0
        else:
            retry_count = retry_val
        exit_code = receipt.get("execution", {}).get("exit_code")
        expected_terminal = receipt.get("expected_terminal")
        observed_terminal = receipt.get("observed_terminal")
        if isinstance(receipt_path, str):
            evidence_paths = [receipt_path]
        else:
            outcome_passed = False
            reasons = reasons + ["receipt_evidence_path_missing"]
            evidence_paths = []

    p0 = _is_p0_failure(entry)
    return {
        "task_id": str(entry.get("task_id", "")),
        "category": str(entry.get("category", "")),
        "receipt_valid": receipt_valid,
        "execution_present": execution_present,
        "outcome_passed": outcome_passed,
        "failure_class": failure_class,
        "reasons": reasons,
        "scope_violation_count": scope_violation_count,
        "resource_leak_count": resource_leak_count,
        "terminal_settlement_count": terminal_settlement_count,
        "retry_count": retry_count,
        "started_at": started_at,
        "ended_at": ended_at,
        "exit_code": exit_code,
        "expected_terminal": expected_terminal,
        "observed_terminal": observed_terminal,
        "evidence_paths": evidence_paths,
        "generation_id": generation_id,
        "p0": p0,
        "waiver": None,
    }


def _acceptance_report(
    report: Mapping[str, object],
    task_runtime: Mapping[str, object],
    generation_id: str,
) -> dict[str, object]:
    """Deep-copy an aggregated report, replacing tasks with acceptance projections, and reseal."""
    new_report = deepcopy(dict(report))
    tasks = list(new_report.get("tasks", []))
    new_tasks = []
    for entry in tasks:
        task_id = entry["task_id"]
        runtime = task_runtime.get(task_id, {})
        acc_task = _acceptance_task(
            entry,
            runtime.get("receipt"),
            runtime.get("receipt_path"),
            runtime.get("started_at"),
            runtime.get("ended_at"),
            generation_id,
        )
        new_tasks.append(acc_task)
    new_report["tasks"] = new_tasks
    return seal(new_report)



@contextlib.contextmanager
def installed_runtime_lease(environ: Mapping[str, str]):
    """Hold a shared generation lease only for the system-installed client."""
    binary = pathlib.Path(environ.get("ALETHEON_BIN", "")).resolve()
    if binary != pathlib.Path("/usr/bin/aletheon"):
        yield
        return
    runtime_root = pathlib.Path(
        environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    ) / "aletheon"
    lock_path = pathlib.Path(
        environ.get(
            "ALETHEON_RUNTIME_LOCK_FILE",
            str(runtime_root / "runtime-mutation.lock"),
        )
    )
    lock_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_SH)
        yield


def validate_official_acceptance_socket(environ: Mapping[str, str]) -> pathlib.Path:
    """Require acceptance to use the invoking user's official Unix socket."""
    runtime_dir = pathlib.Path(
        environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    )
    expected = pathlib.Path(os.path.abspath(runtime_dir / "aletheon/aletheon.sock"))
    configured = environ.get("ALETHEON_ACCEPTANCE_SOCKET", "")
    if not configured:
        raise AcceptanceContractError(
            "ALETHEON_ACCEPTANCE_SOCKET is required in acceptance mode"
        )
    actual = pathlib.Path(os.path.abspath(configured))
    if actual != expected:
        raise AcceptanceContractError(
            "ALETHEON_ACCEPTANCE_SOCKET must name the official user socket"
        )
    try:
        mode = actual.stat().st_mode
    except OSError:
        raise AcceptanceContractError(
            "official acceptance socket is unavailable"
        ) from None
    if not stat.S_ISSOCK(mode):
        raise AcceptanceContractError(
            "official acceptance path is not a Unix socket"
        )
    return actual


def _invalid_entry(task_id: str, reason: str = "receipt_invalid") -> dict[str, object]:
    return {
        "task_id": task_id,
        "category": "unknown",
        "expected_terminal": "unavailable",
        "observed_terminal": "unavailable",
        "outcome_passed": False,
        "completed_engineering_task": False,
        "failure_class": "infrastructure_failure",
        "reasons": [reason],
        "false_success": False,
        "evidence_complete": False,
        "scope_violation": False,
        "resource_leak": False,
        "receipt_path": None,
        "receipt_integrity": None,
        "receipt_valid": False,
    }


def _load_receipt(path: pathlib.Path) -> tuple[dict[str, object], dict | None]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        entry = _invalid_entry(path.stem)
        entry["receipt_path"] = path.name
        return entry, None
    if not isinstance(value, dict) or value.get("schema_version") != 2:
        entry = _invalid_entry(path.stem)
        entry["receipt_path"] = path.name
        return entry, None
    if value.get("task_id") != path.stem:
        entry = _invalid_entry(path.stem)
        entry["receipt_path"] = path.name
        return entry, None

    replayed, message = verify_receipt(value)
    declared_failure = value.get("failure", {})
    declared_class = declared_failure.get("class")
    authoritative_failure = (
        isinstance(declared_class, str)
        and declared_class != "none"
        and message == declared_class
    )
    if not replayed and not authoritative_failure:
        entry = _invalid_entry(path.stem)
        entry["receipt_path"] = path.name
        return entry, None

    operation_id = value["operation_id"]
    evidence = value["evidence"]
    evidence_complete = bool(
        evidence
        and all(item.get("operation_id") == operation_id for item in evidence)
        and any(item.get("kind") == "terminal_snapshot" for item in evidence)
        and any(item.get("kind") == "acceptance_command" for item in evidence)
    )
    outcome_passed = bool(value["verification"]["passed"])
    failure_reasons = list(declared_failure["reasons"])
    # A missing required edit is a task-completeness failure, not proof that
    # the agent crossed a scope boundary. Only forbidden writes or damage to
    # the caller's pre-existing dirty patch count against the zero-scope-
    # violation release gate.
    scope_violation_reasons = {
        "forbidden_path_changed",
        "dirty_patch_not_preserved",
    }
    entry = {
        "task_id": value["task_id"],
        "category": value["category"],
        "expected_terminal": value["expected_terminal"],
        "observed_terminal": value["observed_terminal"],
        "outcome_passed": outcome_passed,
        "completed_engineering_task": outcome_passed
        and value["expected_terminal"] == "verified",
        "failure_class": declared_class,
        "reasons": failure_reasons,
        "false_success": value["execution"]["reported_success"] is True
        and declared_class != "none",
        "evidence_complete": evidence_complete,
        "scope_violation": bool(
            scope_violation_reasons.intersection(failure_reasons)
        ),
        "resource_leak": value["resources"]["passed"] is False,
        "receipt_path": path.name,
        "receipt_integrity": value["integrity_sha256"],
        "receipt_valid": True,
    }
    return entry, value


def _nearest_rank(values: list[int], percentile: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def _metric_summary(values: list[int | None]) -> dict[str, object]:
    available = [value for value in values if value is not None]
    return {
        "available": len(available),
        "unavailable": len(values) - len(available),
        "average": round(sum(available) / len(available), 3) if available else None,
        "p50": _nearest_rank(available, 0.50),
        "p95": _nearest_rank(available, 0.95),
    }


def _category_counts(entries: Sequence[Mapping[str, object]]) -> dict[str, dict[str, int]]:
    categories: dict[str, dict[str, int]] = {}
    for entry in entries:
        category = str(entry["category"])
        counts = categories.setdefault(category, {"total": 0, "passed": 0, "failed": 0})
        counts["total"] += 1
        counts["passed" if entry["outcome_passed"] else "failed"] += 1
    return {category: categories[category] for category in sorted(categories)}


def _build_report(
    entries: list[dict[str, object]],
    receipts: list[dict],
    *,
    catalog_digest: str | None = None,
) -> dict[str, object]:
    entries.sort(key=lambda item: str(item["task_id"]))
    total = len(entries)
    passed = sum(bool(entry["outcome_passed"]) for entry in entries)
    evidence_passed = sum(
        bool(entry["outcome_passed"] and entry["evidence_complete"])
        for entry in entries
    )
    failure_counts = Counter(str(entry["failure_class"]) for entry in entries)
    terminal_counts = Counter(str(entry["observed_terminal"]) for entry in entries)
    if catalog_digest is None:
        task_contract = [
            {
                "task_id": entry["task_id"],
                "category": entry["category"],
                "expected_terminal": entry["expected_terminal"],
            }
            for entry in entries
        ]
        catalog_digest = digest(
            json.dumps(
                task_contract,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            ).encode("utf-8")
        )
    binaries = {
        (value["binary"]["path"], value["binary"]["sha256"])
        for value in receipts
    }
    metrics = {
        metric: _metric_summary([value["metrics"][metric] for value in receipts])
        for metric in sorted(METRIC_KEYS)
    }
    report: dict[str, object] = {
        "schema_version": REPORT_SCHEMA_VERSION,
        "receipt_schema_version": 2,
        "catalog": {
            "task_schema_version": 1,
            "task_ids": [entry["task_id"] for entry in entries],
            "digest": catalog_digest,
        },
        "binaries": [
            {"path": path, "sha256": sha256}
            for path, sha256 in sorted(binaries)
        ],
        "summary": {
            "total": total,
            "benchmark_outcomes_passed": passed,
            "benchmark_outcomes_failed": total - passed,
            "completed_engineering_tasks": sum(
                bool(entry["completed_engineering_task"]) for entry in entries
            ),
            "expected_non_success_outcomes": sum(
                bool(entry["outcome_passed"])
                and entry["expected_terminal"] != "verified"
                for entry in entries
            ),
            "validation_pass_rate": round(passed / total, 6) if total else 0.0,
            "evidence_complete_success_rate": round(evidence_passed / total, 6)
            if total
            else 0.0,
            "false_success_count": sum(bool(entry["false_success"]) for entry in entries),
            "scope_violation_count": sum(
                bool(entry["scope_violation"]) for entry in entries
            ),
            "leaked_resource_count": sum(
                bool(entry["resource_leak"]) for entry in entries
            ),
            "by_category": _category_counts(entries),
            "by_failure_class": {
                key: failure_counts[key] for key in sorted(FAILURE_CLASSES)
            },
            "by_observed_terminal": {
                key: terminal_counts[key] for key in sorted(terminal_counts)
            },
        },
        "metrics": metrics,
        "tasks": entries,
    }
    return seal(report)


def aggregate_receipts(paths: Sequence[pathlib.Path]) -> dict[str, object]:
    loaded = [_load_receipt(path) for path in sorted(paths, key=lambda item: item.name)]
    return _build_report(
        [entry for entry, _ in loaded],
        [value for _, value in loaded if value is not None],
    )


def exit_code(report: Mapping[str, object]) -> int:
    tasks = report.get("tasks", [])
    if any(task.get("failure_class") == "infrastructure_failure" for task in tasks):
        return 2
    if any(not task.get("outcome_passed") for task in tasks):
        return 1
    return 0


def _write_report(report: Mapping[str, object], path: pathlib.Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )





def execute_suite(
    catalog: pathlib.Path,
    receipts_directory: pathlib.Path,
    report_path: pathlib.Path,
    *,
    root: pathlib.Path = ROOT,
    environ: Mapping[str, str] | None = None,
    acceptance_generation_id: str | None = None,
    acceptance_runtime_out: dict[str, object] | None = None,
) -> tuple[dict[str, object], int]:
    environment = dict(os.environ if environ is None else environ)
    with installed_runtime_lease(environment):
        return _execute_suite_locked(
            catalog,
            receipts_directory,
            report_path,
            root=root,
            environ=environment,
            acceptance_generation_id=acceptance_generation_id,
            acceptance_runtime_out=acceptance_runtime_out,
        )


def _execute_suite_locked(
    catalog: pathlib.Path,
    receipts_directory: pathlib.Path,
    report_path: pathlib.Path,
    *,
    root: pathlib.Path = ROOT,
    environ: Mapping[str, str] | None = None,
    acceptance_generation_id: str | None = None,
    acceptance_runtime_out: dict[str, object] | None = None,
) -> tuple[dict[str, object], int]:
    try:
        tasks = load_catalog(_catalog_task_paths(root, catalog), root)
        if not tasks:
            raise ContractError("catalog is empty")
        catalog_digest = acceptance_input_digest(tasks, root)
    except ContractError:
        report = _build_report([_invalid_entry("catalog", "catalog_invalid")], [])
        _write_report(report, report_path)
        return report, 2

    receipts_directory.mkdir(parents=True, exist_ok=True)
    if acceptance_runtime_out is not None:
        if not acceptance_generation_id or not isinstance(acceptance_generation_id, str) or not acceptance_generation_id.strip():
            raise ContractError(
                "acceptance_generation_id must be a non-empty string when acceptance_runtime_out is provided"
            )
    runner_errors: list[BenchmarkTask] = []
    task_run_info: list[dict[str, object]] = []
    for task in tasks:
        start = utc_now()
        receipt_path = None
        try:
            path = receipts_directory / f"{task.id}.json"
            path.unlink(missing_ok=True)
            receipt = run_task(task.source, path, root=root, environ=environ)
            receipt_path = path
        except Exception:
            runner_errors.append(task)
            receipt = None
        end = utc_now()
        task_run_info.append(
            {
                "task_id": task.id,
                "category": task.category,
                "expected_terminal": task.expected_terminal,
                "start": start,
                "end": end,
                "receipt_path": receipt_path,
                "receipt": receipt,
            }
        )

    entries = []
    values = []
    valid_receipts = {}
    for info in task_run_info:
        if info["receipt_path"] is not None:
            entry, receipt_value = _load_receipt(info["receipt_path"])
            if receipt_value is not None and (
                receipt_value.get("category") != info["category"]
                or receipt_value.get("expected_terminal")
                != info["expected_terminal"]
            ):
                entry = _invalid_entry(
                    str(info["task_id"]), "receipt_task_contract_mismatch"
                )
                entry["receipt_path"] = info["receipt_path"].name
                receipt_value = None
            entry["category"] = info["category"]
            entry["expected_terminal"] = info["expected_terminal"]
            entries.append(entry)
            valid_receipts[info["task_id"]] = receipt_value
            if receipt_value is not None:
                values.append(receipt_value)
    for task in runner_errors:
        entry = _invalid_entry(task.id, "runner_exception")
        entry["category"] = task.category
        entry["expected_terminal"] = task.expected_terminal
        entries.append(entry)
    if acceptance_runtime_out is not None:
        runtime_dict: dict[str, object] = {}
        for info in task_run_info:
            tid = str(info["task_id"])
            if info["receipt_path"] is None:
                runtime_dict[tid] = {
                    "receipt": None,
                    "receipt_path": None,
                    "started_at": None,
                    "ended_at": None,
                }
            else:
                receipt_val = valid_receipts.get(tid)
                if receipt_val is None:
                    name = info["receipt_path"].name
                    runtime_dict[tid] = {
                        "receipt": None,
                        "receipt_path": f"logs/{name}",
                        "started_at": info["start"],
                        "ended_at": info["end"],
                    }
                else:
                    name = info["receipt_path"].name
                    if "/" in name or "\\" in name or name.startswith(".") or any(
                        ord(c) < 32 or ord(c) == 127 for c in name
                    ):
                        runtime_dict[tid] = {
                            "receipt": None,
                            "receipt_path": None,
                            "started_at": None,
                            "ended_at": None,
                        }
                    else:
                        runtime_dict[tid] = {
                            "receipt": receipt_val,
                            "receipt_path": f"logs/{name}",
                            "started_at": info["start"],
                            "ended_at": info["end"],
                        }
        acceptance_runtime_out.update(runtime_dict)
    report = _build_report(entries, values, catalog_digest=catalog_digest)
    _write_report(report, report_path)
    return report, exit_code(report)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=pathlib.Path, required=True)
    parser.add_argument("--receipts", type=pathlib.Path, required=True)
    parser.add_argument("--report", type=pathlib.Path, required=True)
    parser.add_argument("--run-id", type=str, default=None)
    parser.add_argument("--generation-id", type=str, default=None)
    parser.add_argument("--artifacts-root", type=pathlib.Path, default=None)
    args = parser.parse_args()
    has_acceptance = any(
        [args.run_id is not None, args.generation_id is not None, args.artifacts_root is not None]
    )
    if has_acceptance:
        if not (args.run_id and args.generation_id and args.artifacts_root):
            parser.error(
                "--run-id, --generation-id, and --artifacts-root must all be supplied together"
            )
        if not isinstance(args.run_id, str) or not args.run_id.strip():
            parser.error("--run-id must be a non-empty string")
        if not isinstance(args.generation_id, str) or not args.generation_id.strip():
            parser.error("--generation-id must be a non-empty string")
        aletheon_bin = pathlib.Path(os.environ.get("ALETHEON_BIN", "")).resolve()
        if aletheon_bin != pathlib.Path("/usr/bin/aletheon"):
            parser.error(
                "fatal: ALETHEON_BIN must resolve to /usr/bin/aletheon in acceptance mode"
            )
        try:
            validate_official_acceptance_socket(os.environ)
        except AcceptanceContractError as error:
            parser.error(f"fatal: {error}")
        catalog = pathlib.Path(os.path.abspath(args.catalog))
        try:
            source_before = capture_acceptance_source_state(ROOT, catalog)
        except (OSError, ContractError):
            parser.error("fatal: acceptance source snapshot failed")
        generation_id = args.generation_id
        runtime_out: dict[str, object] = {}
        report, code = execute_suite(
            catalog,
            args.receipts.resolve(),
            args.report.resolve(),
            environ=os.environ,
            acceptance_generation_id=generation_id,
            acceptance_runtime_out=runtime_out,
        )
        acceptance_report = _acceptance_report(report, runtime_out, generation_id)
        catalog_digest = report.get("catalog", {}).get("digest")
        if not isinstance(catalog_digest, str) or not catalog_digest.startswith("sha256:"):
            parser.error("fatal: report catalog digest missing or malformed")
        hex_digest = catalog_digest[len("sha256:"):]
        if len(hex_digest) != 64 or not all(c in "0123456789abcdef" for c in hex_digest):
            parser.error("fatal: report catalog digest must be sha256: + 64 lowercase hex")
        if catalog_digest != source_before["input_digest"]:
            parser.error("fatal: report does not match initial acceptance inputs")
        try:
            source_after = capture_acceptance_source_state(ROOT, catalog)
            assert_acceptance_source_unchanged(source_before, source_after)
        except (OSError, ContractError):
            parser.error("fatal: acceptance source changed during execution")
        try:
            provenance = collect_installed_provenance(ROOT, hex_digest, generation_id)
        except ProvenanceCollectionError:
            parser.error("fatal: provenance collection failed")
        try:
            assert_acceptance_provenance_matches_source(source_before, provenance)
            source_final = capture_acceptance_source_state(ROOT, catalog)
            assert_acceptance_source_unchanged(source_before, source_final)
        except (OSError, ContractError):
            parser.error("fatal: acceptance provenance/source mismatch")
        try:
            run_dir = write_run(
                args.run_id, acceptance_report, provenance, args.artifacts_root,
                evidence_root=args.receipts.resolve(),
            )
        except (OSError, AcceptanceContractError):
            parser.error("fatal: acceptance artifact write failed")
        print(args.report)
        print(run_dir)
        raise SystemExit(code)
    else:
        report, code = execute_suite(
            pathlib.Path(os.path.abspath(args.catalog)),
            args.receipts.resolve(),
            args.report.resolve(),
            environ=os.environ,
        )
        print(args.report)
        raise SystemExit(code)


if __name__ == "__main__":
    main()
