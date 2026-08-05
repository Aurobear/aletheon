#!/usr/bin/env python3
"""Sequential coding benchmark execution and deterministic suite aggregation."""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import math
import os
import pathlib
from collections import Counter
from collections.abc import Mapping, Sequence

from contracts import BenchmarkTask, ContractError, load_catalog
from receipt import FAILURE_CLASSES, METRIC_KEYS, digest, seal, verify_receipt
from run import ROOT, run_task

REPORT_SCHEMA_VERSION = 1


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
    entry = {
        "task_id": value["task_id"],
        "category": value["category"],
        "expected_terminal": value["expected_terminal"],
        "observed_terminal": value["observed_terminal"],
        "outcome_passed": outcome_passed,
        "completed_engineering_task": outcome_passed
        and value["expected_terminal"] == "verified",
        "failure_class": declared_class,
        "reasons": list(declared_failure["reasons"]),
        "false_success": value["execution"]["reported_success"] is True
        and declared_class != "none",
        "evidence_complete": evidence_complete,
        "scope_violation": declared_class == "policy_scope_failure",
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
    entries: list[dict[str, object]], receipts: list[dict]
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
    task_contract = [
        {
            "task_id": entry["task_id"],
            "category": entry["category"],
            "expected_terminal": entry["expected_terminal"],
        }
        for entry in entries
    ]
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
            "digest": digest(
                json.dumps(
                    task_contract,
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=False,
                ).encode("utf-8")
            ),
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
) -> tuple[dict[str, object], int]:
    environment = dict(os.environ if environ is None else environ)
    with installed_runtime_lease(environment):
        return _execute_suite_locked(
            catalog,
            receipts_directory,
            report_path,
            root=root,
            environ=environment,
        )


def _execute_suite_locked(
    catalog: pathlib.Path,
    receipts_directory: pathlib.Path,
    report_path: pathlib.Path,
    *,
    root: pathlib.Path = ROOT,
    environ: Mapping[str, str] | None = None,
) -> tuple[dict[str, object], int]:
    try:
        tasks = load_catalog(sorted(catalog.glob("*.toml")), root)
        if not tasks:
            raise ContractError("catalog is empty")
    except ContractError:
        report = _build_report([_invalid_entry("catalog", "catalog_invalid")], [])
        _write_report(report, report_path)
        return report, 2

    receipts_directory.mkdir(parents=True, exist_ok=True)
    receipt_paths: list[pathlib.Path] = []
    runner_errors: list[BenchmarkTask] = []
    for task in tasks:
        path = receipts_directory / f"{task.id}.json"
        path.unlink(missing_ok=True)
        try:
            run_task(task.source, path, root=root, environ=environ)
            receipt_paths.append(path)
        except Exception:
            runner_errors.append(task)

    loaded = [_load_receipt(path) for path in receipt_paths]
    entries = [entry for entry, _ in loaded]
    values = [value for _, value in loaded if value is not None]
    for task in runner_errors:
        entry = _invalid_entry(task.id, "runner_exception")
        entry["category"] = task.category
        entry["expected_terminal"] = task.expected_terminal
        entries.append(entry)
    report = _build_report(entries, values)
    _write_report(report, report_path)
    return report, exit_code(report)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=pathlib.Path, required=True)
    parser.add_argument("--receipts", type=pathlib.Path, required=True)
    parser.add_argument("--report", type=pathlib.Path, required=True)
    arguments = parser.parse_args()
    report, code = execute_suite(
        arguments.catalog.resolve(),
        arguments.receipts.resolve(),
        arguments.report.resolve(),
        environ=os.environ,
    )
    print(arguments.report)
    raise SystemExit(code)


if __name__ == "__main__":
    main()
