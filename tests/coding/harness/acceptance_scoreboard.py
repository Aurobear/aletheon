"""Acceptance scoreboard construction and validation."""

import copy
import re
from collections.abc import Mapping
from datetime import datetime, timedelta, timezone

from acceptance_contract import (
    ALLOWED_STATUSES,
    ContractError,
    ENVIRONMENT_LEVELS,
    validate_provenance,
    validate_waiver,
)
from receipt import (
    EXPECTED_TERMINALS,
    OBSERVED_TERMINALS,
    verify_integrity as receipt_verify_integrity,
)

_TS_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
_RUN_ID_RE = re.compile(r"^[A-Za-z0-9._-]+$")
_REPORT_KEYS = {
    "schema_version", "receipt_schema_version", "catalog", "binaries",
    "summary", "metrics", "tasks", "integrity_sha256",
}
_ENTRY_KEYS = {
    "task_id", "category", "receipt_valid", "execution_present",
    "outcome_passed", "failure_class", "reasons",
    "scope_violation_count", "resource_leak_count",
    "terminal_settlement_count", "retry_count", "started_at",
    "ended_at", "exit_code", "expected_terminal", "observed_terminal",
    "evidence_paths", "generation_id", "p0", "waiver",
}

EXPECTED_TERMINAL_EXIT_CODES = {
    "verified": 0,
    "blocked": 20,
    "budget_exhausted": 20,
    "cancelled": 21,
}

TERMINAL_STATUSES = {"passed", "failed", "waived"}
VALID_GATE_STATUSES = TERMINAL_STATUSES | {"not_run", "infra_blocked"}


def _fail(message):
    raise ContractError(message)


def _require_mapping(value, name):
    if not isinstance(value, Mapping):
        _fail(f"{name} must be a mapping")


def _nonempty_str(value, name):
    if not isinstance(value, str) or not value:
        _fail(f"{name} must be a nonempty string")
    try:
        size = len(value.encode("utf-8"))
    except UnicodeEncodeError:
        _fail(f"{name} must be valid UTF-8")
    if size > 4096:
        _fail(f"{name} must be at most 4096 bytes")
    return value


def _str_list(value, name):
    if not isinstance(value, list):
        _fail(f"{name} must be a list")
    if len(value) > 128:
        _fail(f"{name} must contain at most 128 entries")
    for item in value:
        _nonempty_str(item, f"{name} entries")


def _count(value, name):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        _fail(f"{name} must be a nonnegative integer")


def _bool(value, name):
    if not isinstance(value, bool):
        _fail(f"{name} must be a bool")


def _parse_timestamp(value, name):
    if not isinstance(value, str) or not _TS_RE.match(value):
        _fail(f"{name} must be a UTC timestamp")
    try:
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        _fail(f"{name} must be a real UTC timestamp")


def _normalize_utc_reference(value, name):
    if not isinstance(value, datetime) or value.tzinfo is None:
        _fail(f"{name} must be a timezone-aware UTC datetime")
    try:
        offset = value.utcoffset()
    except Exception:
        _fail(f"{name} must be a timezone-aware UTC datetime")
    if offset is None or offset != timedelta(0):
        _fail(f"{name} must be a timezone-aware UTC datetime")
    try:
        return value.astimezone(timezone.utc).replace(microsecond=0)
    except Exception:
        _fail(f"{name} must be a timezone-aware UTC datetime")


def _normalize_digest(value):
    if not isinstance(value, str):
        _fail("digest must be a string")
    if value.startswith("sha256:"):
        value = value[len("sha256:"):]
    if len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        _fail("digest must be sha256:")
    return "sha256:" + value


def _validate_evidence_paths(paths, execution_present):
    if not isinstance(paths, list):
        _fail("evidence_paths must be a list")
    if len(paths) > 128:
        _fail("evidence_paths must have at most 128 entries")
    seen = set()
    for ep in paths:
        if not isinstance(ep, str) or not ep:
            _fail("evidence_paths entries must be nonempty strings")
        if len(ep) > 128:
            _fail("evidence_paths entries must be at most 128 characters")
        # UTF-8 byte length <= 4096
        if len(ep.encode("utf-8")) > 4096:
            _fail("evidence_path must not exceed 4096 UTF-8 bytes")
        if '\\' in ep:
            _fail("evidence_path must not contain backslash")
        if any(ord(c) < 32 or ord(c) == 127 for c in ep):
            _fail("evidence_path must not contain control characters")
        if ep.startswith('/'):
            _fail("evidence_path must not be absolute")
        if ep.endswith('/'):
            _fail("evidence_path must not end with a trailing slash")
        if not ep.startswith("logs/"):
            _fail("evidence_path must be under logs/")
        parts = ep.split('/')
        for part in parts:
            if part == '':
                _fail("evidence_path must not contain empty segments")
            if part in ('.', '..'):
                _fail("evidence_path must not contain dot or dot-dot segments")
        if ep in seen:
            _fail("evidence_paths must be unique")
        seen.add(ep)
    if not execution_present and paths:
        _fail("not_run tasks must have empty evidence_paths")
    if execution_present and not paths:
        _fail("executed tasks must have evidence_paths")


def _waiver_allowed(entry, reference_time):
    waiver = entry["waiver"]
    if waiver is None:
        return False
    validate_waiver(waiver, p0=entry["p0"], reference_time=reference_time)
    return True


def project_task(entry, generation_id, reference_time=None):
    """Project a single task entry to a status-bearing task dict.

    Status rules (audit requirement 3):
      - not executed => not_run with null timing/exit/evidence, zero counts
      - executed with infrastructure_failure => infra_blocked
      - executed invalid receipt => failed
      - passed only if the receipt's expected and observed terminals match,
        the client exit code is canonical for that terminal, outcome is true,
        receipt is valid, settlement is 1, scope is 0, and leak is 0
      - otherwise failed
      - valid non-P0 waiver can convert failed/infra_blocked/not_run to waived;
        P0 waiver rejected.
    """
    if not isinstance(entry, Mapping) or set(entry.keys()) != _ENTRY_KEYS:
        _fail("entry must contain exactly the required task keys")
    _nonempty_str(generation_id, "generation_id")
    _nonempty_str(entry["task_id"], "task_id")
    _nonempty_str(entry["category"], "category")
    _nonempty_str(entry["failure_class"], "failure_class")
    _str_list(entry["reasons"], "reasons")
    _str_list(entry["evidence_paths"], "evidence_paths")
    _count(entry["scope_violation_count"], "scope_violation_count")
    _count(entry["resource_leak_count"], "resource_leak_count")
    _count(entry["terminal_settlement_count"], "terminal_settlement_count")
    _count(entry["retry_count"], "retry_count")
    if entry["exit_code"] is not None and (
        isinstance(entry["exit_code"], bool) or not isinstance(entry["exit_code"], int)
    ):
        _fail("exit_code must be an integer or None")
    if entry["expected_terminal"] not in EXPECTED_TERMINALS:
        _fail("expected_terminal must be a supported expected terminal")
    if (
        entry["observed_terminal"] is not None
        and entry["observed_terminal"] not in OBSERVED_TERMINALS
    ):
        _fail("observed_terminal must be a supported observed terminal or None")
    _bool(entry["receipt_valid"], "receipt_valid")
    _bool(entry["execution_present"], "execution_present")
    _bool(entry["outcome_passed"], "outcome_passed")
    _bool(entry["p0"], "p0")
    if entry["generation_id"] != generation_id:
        _fail("task generation does not match provenance generation")

    _validate_evidence_paths(entry["evidence_paths"], entry["execution_present"])

    if not entry["execution_present"]:
        if entry["outcome_passed"]:
            _fail("cannot report outcome when task was not executed")
        if entry["receipt_valid"] is not False:
            _fail("not_run tasks must have receipt_valid false")
        if entry["started_at"] is not None or entry["ended_at"] is not None:
            _fail("not_run tasks must have None start/end timestamps")
        if entry["exit_code"] is not None:
            _fail("not_run tasks must have None exit_code")
        if entry["observed_terminal"] is not None:
            _fail("not_run tasks must have None observed_terminal")
        if (
            entry["terminal_settlement_count"] != 0
            or entry["retry_count"] != 0
            or entry["scope_violation_count"] != 0
            or entry["resource_leak_count"] != 0
        ):
            _fail("not_run tasks must have zero settlement/retry/scope/leak counts")
        status = "not_run"
        if _waiver_allowed(entry, reference_time):
            status = "waived"
    else:
        if entry["started_at"] is None or entry["ended_at"] is None:
            _fail("executed tasks must have start and end timestamps")
        start = _parse_timestamp(entry["started_at"], "started_at")
        end = _parse_timestamp(entry["ended_at"], "ended_at")
        if end < start:
            _fail("ended_at must not be before started_at")
        if entry["exit_code"] is None:
            _fail("executed tasks must have an integer exit_code")
        if entry["receipt_valid"] and entry["observed_terminal"] is None:
            _fail("valid executed tasks must have an observed_terminal")
        if entry["failure_class"] == "infrastructure_failure":
            status = "infra_blocked"
        elif not entry["receipt_valid"]:
            status = "failed"
        elif entry["failure_class"] == "none":
            if (
                entry["outcome_passed"] is True
                and entry["observed_terminal"] == entry["expected_terminal"]
                and entry["exit_code"]
                == EXPECTED_TERMINAL_EXIT_CODES[entry["expected_terminal"]]
                and entry["receipt_valid"] is True
                and entry["terminal_settlement_count"] == 1
                and entry["scope_violation_count"] == 0
                and entry["resource_leak_count"] == 0
            ):
                status = "passed"
            else:
                status = "failed"
        else:
            status = "failed"

        if status in ("failed", "infra_blocked") and _waiver_allowed(entry, reference_time):
            status = "waived"

    return {
        "task_id": entry["task_id"],
        "category": entry["category"],
        "status": status,
        "receipt_valid": entry["receipt_valid"],
        "execution_present": entry["execution_present"],
        "outcome_passed": entry["outcome_passed"],
        "failure_class": entry["failure_class"],
        "reasons": list(entry["reasons"]),
        "scope_violation_count": entry["scope_violation_count"],
        "resource_leak_count": entry["resource_leak_count"],
        "terminal_settlement_count": entry["terminal_settlement_count"],
        "retry_count": entry["retry_count"],
        "started_at": entry["started_at"],
        "ended_at": entry["ended_at"],
        "exit_code": entry["exit_code"],
        "expected_terminal": entry["expected_terminal"],
        "observed_terminal": entry["observed_terminal"],
        "evidence_paths": list(entry["evidence_paths"]),
        "generation_id": generation_id,
        "p0": entry["p0"],
        "waiver": entry["waiver"],
    }


def validate_projected_task(task, generation_id, evaluated_at):
    """Validate that a projected task matches the projection computed from its entry.

    The task dict must include a 'status' key; the function strips it,
    recomputes the projection using the common ``project_task`` function, and
    compares the two projections.  ``evaluated_at`` is a timezone-aware UTC
    datetime used as the reference time for waiver checks.

    Raises ContractError on any structural mismatch or projection mismatch.
    The error message only includes the sanitized ``task_id``.
    """
    required_keys = {
        "task_id", "category", "receipt_valid", "execution_present",
        "outcome_passed", "failure_class", "reasons",
        "scope_violation_count", "resource_leak_count",
        "terminal_settlement_count", "retry_count", "started_at",
        "ended_at", "exit_code", "expected_terminal", "observed_terminal",
        "evidence_paths", "generation_id", "p0", "waiver", "status",
    }
    if not isinstance(task, dict) or set(task.keys()) != required_keys:
        _fail("projected task must have exact required keys including status")
    if task["generation_id"] != generation_id:
        _fail("generation_id mismatch")

    evaluated_at = _normalize_utc_reference(evaluated_at, "evaluated_at")

    task_id = task.get("task_id")
    if not isinstance(task_id, str) or not task_id:
        _fail("task_id must be a nonempty string")

    entry = {k: v for k, v in task.items() if k != "status"}
    expected = project_task(entry, generation_id, evaluated_at)
    if expected != task:
        raise ContractError(f"projected task mismatch for task_id {task_id}")
    return expected


def gate_verdict(tasks):
    """Compute gate verdict from projected tasks.

    Defensively validates the task list and raises ContractError for
    structurally malformed entries (non-list, non-dict, missing keys,
    wrong types, unknown status enum).

    Valid but non-passing conditions produce a ``failed`` verdict with
    deterministic reasons:
      - task_count != 20
      - any non-terminal status (not_run/infra_blocked)
      - any terminal_settlement_count != 1
      - insufficient passed count (<19)
      - P0 task non-passed
      - scope / resource leak > 0
      - generation mismatch (not exactly one nonempty generation id)
    """
    if not isinstance(tasks, list):
        raise ContractError("tasks must be a list")

    # ---- structural validation (malformed -> exception) ----
    for i, t in enumerate(tasks):
        if not isinstance(t, dict):
            raise ContractError(f"task {i} must be a dict")
        required = {
            "status", "p0", "scope_violation_count",
            "resource_leak_count", "terminal_settlement_count",
            "generation_id",
        }
        missing = required - set(t.keys())
        if missing:
            raise ContractError(f"task {i} missing required keys: {sorted(missing)}")
        status = t["status"]
        if not isinstance(status, str) or status not in VALID_GATE_STATUSES:
            raise ContractError(f"task {i} has invalid status {status!r}")
        if not isinstance(t["p0"], bool):
            raise ContractError(f"task {i} p0 must be a bool")
        for cnt_key in ("scope_violation_count", "resource_leak_count", "terminal_settlement_count"):
            val = t[cnt_key]
            if isinstance(val, bool) or not isinstance(val, int) or val < 0:
                raise ContractError(f"task {i} {cnt_key} must be a nonnegative integer")
        gen = t["generation_id"]
        if not isinstance(gen, str) or not gen:
            raise ContractError(f"task {i} generation_id must be a nonempty string")

    # ---- gate logic (non-passing conditions become reasons, not exceptions) ----
    reasons = []

    if len(tasks) != 20:
        reasons.append("task_count")

    if any(t["status"] not in TERMINAL_STATUSES for t in tasks):
        reasons.append("nonterminal_tasks")

    if any(t["terminal_settlement_count"] != 1 for t in tasks):
        reasons.append("settlement_mismatch")

    passed_count = sum(1 for t in tasks if t["status"] == "passed")
    if passed_count < 19:
        reasons.append("insufficient_passed")

    p0_nonpassed = [t for t in tasks if t["p0"] and t["status"] != "passed"]
    if p0_nonpassed:
        reasons.append("p0_failure")

    if any(t["scope_violation_count"] > 0 for t in tasks):
        reasons.append("scope_violation")

    if any(t["resource_leak_count"] > 0 for t in tasks):
        reasons.append("resource_leak")

    generation_ids = {t["generation_id"] for t in tasks}
    if len(generation_ids) != 1 or any(not g for g in generation_ids):
        reasons.append("generation_mismatch")

    verdict_status = "passed" if not reasons else "failed"
    return {"status": verdict_status, "reasons": sorted(set(reasons))}


def _verify_binaries(binaries, installed_digest):
    if not isinstance(binaries, list) or not binaries:
        _fail("binaries must be a nonempty list")
    normalized_installed = _normalize_digest(installed_digest)
    seen_paths = set()
    result = []
    for i, entry in enumerate(binaries):
        if not isinstance(entry, Mapping) or set(entry.keys()) != {"path", "sha256"}:
            _fail(f"binaries[{i}] must have exact keys path, sha256")
        p = _nonempty_str(entry["path"], f"binaries[{i}].path")
        if p in seen_paths:
            _fail(f"duplicate binary path {p}")
        seen_paths.add(p)
        d = _normalize_digest(entry["sha256"])
        if d != normalized_installed:
            _fail(f"binaries[{i}] sha256 does not match installed_artifact")
        result.append({"path": p, "sha256": d})
    return result


def build_scoreboard(run_id, report, provenance, reference_time=None):
    """Validate and build a scoreboard from a report and provenance.

    Parameters:
        run_id: safe ASCII identifier with length 1..128.
        report: the sealed acceptance report.
        provenance: the provenance record.
        reference_time: optional UTC aware datetime used for waiver expiry checks.
                        Defaults to datetime.now(timezone.utc) once at call time.
    Returns:
        A scoreboard dict that includes an ``evaluated_at`` UTC timestamp.
    """
    if not isinstance(run_id, str) or not _RUN_ID_RE.fullmatch(run_id):
        raise ContractError("run_id must be a safe ASCII identifier")
    if len(run_id) < 1 or len(run_id) > 128:
        raise ContractError("run_id length must be 1..128")

    _require_mapping(report, "report")
    if set(report.keys()) != _REPORT_KEYS:
        _fail("report must contain exactly the required report keys")
    if report.get("schema_version") != 1:
        _fail("unsupported report schema_version")
    if report.get("receipt_schema_version") != 2:
        _fail("unsupported receipt_schema_version")
    _normalize_digest(report["integrity_sha256"])

    # --- Cryptographic integrity verification of the report seal (done first) ---
    if not receipt_verify_integrity(report):
        _fail("report integrity seal is invalid")
    # ---------------------------------------------------------------------------

    _require_mapping(report["catalog"], "catalog")
    if set(report["catalog"].keys()) != {"task_schema_version", "task_ids", "digest"}:
        _fail("catalog must have the exact current catalog keys")
    if report["catalog"].get("task_schema_version") != 1:
        _fail("unsupported catalog task_schema_version")
    catalog_task_ids = report["catalog"]["task_ids"]
    if not isinstance(catalog_task_ids, list) or not catalog_task_ids:
        _fail("catalog task_ids must be a nonempty list")
    if len(catalog_task_ids) != len(set(catalog_task_ids)):
        _fail("catalog task_ids must be unique")
    if not all(isinstance(tid, str) and 0 < len(tid) <= 128 for tid in catalog_task_ids):
        _fail("catalog task_ids must be bounded strings")

    _require_mapping(report["metrics"], "metrics")
    _require_mapping(report["summary"], "summary")
    if not isinstance(report["tasks"], list):
        _fail("report tasks must be a list")

    _require_mapping(provenance, "provenance")
    validated_provenance = validate_provenance(provenance)
    generation_id = validated_provenance["generation_id"]

    installed_artifact = validated_provenance.get("installed_artifact")
    if not isinstance(installed_artifact, Mapping) or not isinstance(installed_artifact.get("sha256"), str):
        _fail("provenance installed_artifact sha256 must be a string")
    normalized_binaries = _verify_binaries(report["binaries"], installed_artifact["sha256"])

    fixture_digest = validated_provenance.get("fixture_digest")
    if _normalize_digest(report["catalog"].get("digest")) != _normalize_digest("sha256:" + fixture_digest):
        _fail("catalog digest does not match provenance fixture digest")

    eval_time = reference_time if reference_time is not None else datetime.now(timezone.utc)
    eval_time = _normalize_utc_reference(eval_time, "reference_time")
    evaluated_at_str = eval_time.strftime("%Y-%m-%dT%H:%M:%SZ")

    projected = [project_task(task, generation_id, eval_time) for task in report["tasks"]]
    task_ids = [t["task_id"] for t in projected]
    if len(task_ids) != len(set(task_ids)):
        _fail("duplicate task IDs are not allowed")
    if sorted(task_ids) != sorted(catalog_task_ids):
        _fail("projected task IDs do not match catalog task_ids")

    gate = gate_verdict(projected)

    return {
        "schema_version": 1,
        "run_id": run_id,
        "evaluated_at": evaluated_at_str,
        "allowed_statuses": list(ALLOWED_STATUSES),
        "environment_levels": list(ENVIRONMENT_LEVELS),
        "provenance": validated_provenance,
        "binaries": normalized_binaries,
        "tasks": projected,
        "summary": copy.deepcopy(report["summary"]),
        "metrics": copy.deepcopy(report["metrics"]),
        "gate": gate,
    }
