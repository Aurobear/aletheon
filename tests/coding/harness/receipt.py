#!/usr/bin/env python3
"""Versioned, fail-closed receipt contracts for the coding benchmark."""
from __future__ import annotations

import hashlib
import json
import re
from collections.abc import Mapping
from typing import Any

SCHEMA_VERSION = 2
TASK_SCHEMA_VERSION = 1

TOP_LEVEL_KEYS = frozenset(
    {
        "schema_version",
        "task_schema_version",
        "task_id",
        "category",
        "binary",
        "operation_id",
        "observed_stop",
        "observed_terminal",
        "expected_terminal",
        "execution",
        "workspace",
        "acceptance",
        "evidence",
        "resources",
        "metrics",
        "failure",
        "verification",
        "integrity_sha256",
    }
)
BINARY_KEYS = frozenset({"path", "sha256"})
EXECUTION_KEYS = frozenset(
    {
        "argv",
        "exit_code",
        "timed_out",
        "elapsed_ms",
        "stdout",
        "stderr",
        "stdout_digest",
        "stderr_digest",
        "stdout_truncated",
        "stderr_truncated",
        "json_valid",
        "terminal_snapshot",
        "reported_success",
        "process_group_reaped",
        "infrastructure_error",
    }
)
COMMAND_KEYS = frozenset(
    {
        "argv",
        "exit_code",
        "timed_out",
        "elapsed_ms",
        "stdout",
        "stderr",
        "stdout_digest",
        "stderr_digest",
        "stdout_truncated",
        "stderr_truncated",
        "process_group_reaped",
    }
)
WORKSPACE_KEYS = frozenset(
    {
        "base_tree_digest",
        "diff",
        "diff_digest",
        "changed_files",
        "forbidden_paths_unchanged",
        "required_scope_satisfied",
        "dirty_patch_digest",
        "dirty_patch_preserved",
    }
)
RESOURCE_KEYS = frozenset({"passed", "checks"})
METRIC_KEYS = frozenset(
    {
        "iterations",
        "tool_calls",
        "tool_errors",
        "inference_rounds",
        "provider_retries",
        "active_context_tokens",
        "elapsed_ms",
    }
)
FAILURE_KEYS = frozenset({"class", "reasons"})
VERIFICATION_KEYS = frozenset({"passed"})

EXPECTED_TERMINALS = frozenset(
    {"verified", "blocked", "budget_exhausted", "cancelled"}
)
OBSERVED_STOPS = frozenset(
    {"completed", "blocked", "cancelled", "failed", "unavailable"}
)
OBSERVED_TERMINALS = frozenset(
    {"verified", "blocked", "budget_exhausted", "cancelled", "failed", "unavailable"}
)
TERMINAL_STOPS = {
    "verified": frozenset({"completed"}),
    "blocked": frozenset({"blocked"}),
    "budget_exhausted": frozenset({"blocked"}),
    "cancelled": frozenset({"cancelled", "unavailable"}),
    "failed": frozenset({"failed"}),
    "unavailable": frozenset({"unavailable"}),
}
FAILURE_CLASSES = frozenset(
    {
        "infrastructure_failure",
        "timeout_or_cancellation",
        "policy_scope_failure",
        "execution_failure",
        "verification_failure",
        "none",
    }
)
_SHA256 = re.compile(r"sha256:[0-9a-f]{64}\Z")
MAX_CAPTURE = 64 * 1024


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def canonical_bytes(value: Mapping[str, Any]) -> bytes:
    body = {key: item for key, item in value.items() if key != "integrity_sha256"}
    return json.dumps(
        body, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def seal(value: dict[str, Any]) -> dict[str, Any]:
    """Replace the integrity field with a canonical SHA-256 seal."""
    value["integrity_sha256"] = digest(canonical_bytes(value))
    return value


def verify_integrity(value: Mapping[str, Any]) -> bool:
    expected = value.get("integrity_sha256")
    return isinstance(expected, str) and expected == digest(canonical_bytes(value))


def _exact_keys(value: Any, expected: frozenset[str], name: str) -> str | None:
    if not isinstance(value, dict):
        return f"{name} must be an object"
    actual = frozenset(value)
    if actual != expected:
        missing = sorted(expected - actual)
        unknown = sorted(actual - expected)
        return f"{name} keys mismatch: missing={missing}, unknown={unknown}"
    return None


def _non_negative_optional_integer(value: Any) -> bool:
    return value is None or (isinstance(value, int) and not isinstance(value, bool) and value >= 0)


def _validate_command(value: Any, name: str, expected_keys: frozenset[str]) -> str | None:
    error = _exact_keys(value, expected_keys, name)
    if error:
        return error
    if not isinstance(value["argv"], list) or not value["argv"] or any(
        not isinstance(argument, str) or not argument for argument in value["argv"]
    ):
        return f"{name}.argv must be a non-empty string list"
    if value["exit_code"] is not None and (
        not isinstance(value["exit_code"], int) or isinstance(value["exit_code"], bool)
    ):
        return f"{name}.exit_code must be an integer or null"
    if not isinstance(value["timed_out"], bool):
        return f"{name}.timed_out must be boolean"
    if not _non_negative_optional_integer(value["elapsed_ms"]):
        return f"{name}.elapsed_ms must be a non-negative integer"
    for stream in ("stdout", "stderr"):
        if not isinstance(value[stream], str) or len(value[stream].encode()) > MAX_CAPTURE:
            return f"{name}.{stream} must be bounded text"
        if not isinstance(value[f"{stream}_digest"], str) or not _SHA256.fullmatch(
            value[f"{stream}_digest"]
        ):
            return f"{name}.{stream}_digest must be a SHA-256 digest"
        if not isinstance(value[f"{stream}_truncated"], bool):
            return f"{name}.{stream}_truncated must be boolean"
    if not isinstance(value["process_group_reaped"], bool):
        return f"{name}.process_group_reaped must be boolean"
    return None


def _validate_v2(value: Any) -> str | None:
    error = _exact_keys(value, TOP_LEVEL_KEYS, "receipt")
    if error:
        return error
    if value["schema_version"] != SCHEMA_VERSION:
        return "unsupported receipt schema version"
    if value["task_schema_version"] != TASK_SCHEMA_VERSION:
        return "unsupported task schema version"
    if not isinstance(value["task_id"], str) or not value["task_id"].strip():
        return "task_id must be non-empty"
    if not isinstance(value["category"], str) or not value["category"].strip():
        return "category must be non-empty"

    error = _exact_keys(value["binary"], BINARY_KEYS, "binary")
    if error:
        return error
    if not isinstance(value["binary"]["path"], str) or not value["binary"]["path"]:
        return "binary.path must be non-empty"
    if not isinstance(value["binary"]["sha256"], str) or not _SHA256.fullmatch(
        value["binary"]["sha256"]
    ):
        return "binary.sha256 must be a SHA-256 digest"

    if not isinstance(value["operation_id"], str):
        return "operation_id must be a string"
    if value["observed_stop"] not in OBSERVED_STOPS:
        return "invalid observed_stop"
    if value["observed_terminal"] not in OBSERVED_TERMINALS:
        return "invalid observed_terminal"
    if value["observed_stop"] not in TERMINAL_STOPS[value["observed_terminal"]]:
        return "observed terminal contradicts authoritative stop"
    if value["expected_terminal"] not in EXPECTED_TERMINALS:
        return "invalid expected_terminal"

    error = _exact_keys(value["execution"], EXECUTION_KEYS, "execution")
    if error:
        return error
    execution = value["execution"]
    error = _validate_command(execution, "execution", EXECUTION_KEYS)
    if error:
        return error
    for field in ("json_valid", "terminal_snapshot"):
        if not isinstance(execution[field], bool):
            return f"execution.{field} must be boolean"
    if execution["reported_success"] is not None and not isinstance(
        execution["reported_success"], bool
    ):
        return "execution.reported_success must be boolean or null"
    if execution["infrastructure_error"] is not None and (
        not isinstance(execution["infrastructure_error"], str)
        or not execution["infrastructure_error"].strip()
    ):
        return "execution.infrastructure_error must be a reason code or null"

    error = _exact_keys(value["workspace"], WORKSPACE_KEYS, "workspace")
    if error:
        return error
    workspace = value["workspace"]
    if not isinstance(workspace["base_tree_digest"], str) or not _SHA256.fullmatch(
        workspace["base_tree_digest"]
    ):
        return "workspace.base_tree_digest must be a SHA-256 digest"
    if not isinstance(workspace["diff"], str):
        return "workspace.diff must be a string"
    if not isinstance(workspace["diff_digest"], str) or not _SHA256.fullmatch(
        workspace["diff_digest"]
    ):
        return "workspace.diff_digest must be a SHA-256 digest"
    if workspace["dirty_patch_digest"] is not None and (
        not isinstance(workspace["dirty_patch_digest"], str)
        or not _SHA256.fullmatch(workspace["dirty_patch_digest"])
    ):
        return "workspace.dirty_patch_digest must be a SHA-256 digest or null"
    if not isinstance(workspace["changed_files"], list) or any(
        not isinstance(path, str) or not path for path in workspace["changed_files"]
    ):
        return "workspace.changed_files must contain paths"
    for field in (
        "forbidden_paths_unchanged",
        "required_scope_satisfied",
        "dirty_patch_preserved",
    ):
        if not isinstance(workspace[field], bool):
            return f"workspace.{field} must be boolean"

    if not isinstance(value["acceptance"], list):
        return "acceptance must be a list of objects"
    for index, item in enumerate(value["acceptance"]):
        error = _validate_command(item, f"acceptance[{index}]", COMMAND_KEYS)
        if error:
            return error
    if not isinstance(value["evidence"], list) or any(
        not isinstance(item, dict) for item in value["evidence"]
    ):
        return "evidence must be a list of objects"

    error = _exact_keys(value["resources"], RESOURCE_KEYS, "resources")
    if error:
        return error
    if not isinstance(value["resources"]["passed"], bool):
        return "resources.passed must be boolean"
    if not isinstance(value["resources"]["checks"], list) or any(
        not isinstance(check, dict)
        or set(check) != {"name", "passed"}
        or not isinstance(check["name"], str)
        or not isinstance(check["passed"], bool)
        for check in value["resources"]["checks"]
    ):
        return "resources.checks must be a list"

    error = _exact_keys(value["metrics"], METRIC_KEYS, "metrics")
    if error:
        return error
    for field, metric in value["metrics"].items():
        if not _non_negative_optional_integer(metric):
            return f"metrics.{field} must be a non-negative integer or null"

    error = _exact_keys(value["failure"], FAILURE_KEYS, "failure")
    if error:
        return error
    if value["failure"]["class"] not in FAILURE_CLASSES:
        return "invalid failure class"
    reasons = value["failure"]["reasons"]
    if not isinstance(reasons, list) or any(
        not isinstance(reason, str) or not reason for reason in reasons
    ):
        return "failure.reasons must contain reason codes"

    error = _exact_keys(value["verification"], VERIFICATION_KEYS, "verification")
    if error:
        return error
    if not isinstance(value["verification"]["passed"], bool):
        return "verification.passed must be boolean"
    if not isinstance(value["integrity_sha256"], str) or not _SHA256.fullmatch(
        value["integrity_sha256"]
    ):
        return "integrity_sha256 must be a SHA-256 digest"
    return None


def classify_failure(value: Mapping[str, Any]) -> tuple[str, list[str]]:
    """Return exactly one failure class using host-owned precedence."""
    execution = value.get("execution", {})
    workspace = value.get("workspace", {})
    metrics = value.get("metrics", {})

    infrastructure: list[str] = []
    explicit = execution.get("infrastructure_error")
    if explicit:
        infrastructure.append(str(explicit))
    if not execution.get("timed_out"):
        if not value.get("operation_id"):
            infrastructure.append("operation_id_missing")
        if execution.get("json_valid") is False:
            infrastructure.append("client_json_invalid")
        if execution.get("terminal_snapshot") is False:
            infrastructure.append("terminal_snapshot_missing")
    if infrastructure:
        return "infrastructure_failure", sorted(set(infrastructure))

    timeout: list[str] = []
    if execution.get("timed_out"):
        timeout.append("execution_timeout")
    if value.get("observed_stop") == "cancelled" and value.get(
        "expected_terminal"
    ) != "cancelled":
        timeout.append("unexpected_cancellation")
    if timeout:
        return "timeout_or_cancellation", timeout

    policy: list[str] = []
    if workspace.get("forbidden_paths_unchanged") is False:
        policy.append("forbidden_path_changed")
    if workspace.get("required_scope_satisfied") is False:
        policy.append("required_scope_not_satisfied")
    if workspace.get("dirty_patch_preserved") is False:
        policy.append("dirty_patch_not_preserved")
    if policy:
        return "policy_scope_failure", policy

    runtime: list[str] = []
    matched_expected_non_success = (
        value.get("expected_terminal") != "verified"
        and value.get("observed_terminal") == value.get("expected_terminal")
    )
    if execution.get("exit_code") not in (0, None) and not matched_expected_non_success:
        runtime.append("client_exit_nonzero")
    if value.get("observed_stop") == "failed":
        runtime.append("authoritative_stop_failed")
    if (
        value.get("observed_stop") == "completed"
        and execution.get("reported_success") is not True
    ):
        runtime.append("client_reported_failure")
    tool_errors = metrics.get("tool_errors")
    if isinstance(tool_errors, int) and tool_errors > 0:
        runtime.append("tool_error_observed")
    if value.get("observed_terminal") != value.get("expected_terminal"):
        runtime.append("unexpected_terminal")
    if runtime:
        return "execution_failure", runtime

    verification: list[str] = []
    operation_id = value.get("operation_id")
    evidence = value.get("evidence", [])
    if not evidence or any(item.get("operation_id") != operation_id for item in evidence):
        verification.append("operation_evidence_mismatch")
    if not execution.get("process_group_reaped"):
        verification.append("process_group_not_reaped")
    resources = value.get("resources", {})
    if not resources.get("passed"):
        verification.append("resource_check_failed")

    acceptance = value.get("acceptance", [])
    if not acceptance or any(
        item.get("exit_code") != 0 or item.get("timed_out") is not False
        for item in acceptance
    ):
        verification.append("acceptance_failed")
    if not any(
        item.get("kind") == "acceptance_command" and item.get("exit_code") == 0
        for item in evidence
    ):
        verification.append("acceptance_evidence_missing")
    if not any(item.get("kind") == "terminal_snapshot" for item in evidence):
        verification.append("terminal_evidence_missing")

    if value.get("expected_terminal") == "verified":
        if not str(workspace.get("diff", "")).strip():
            verification.append("workspace_diff_missing")

    if verification:
        return "verification_failure", verification
    return "none", []


def verify_receipt(value: Any) -> tuple[bool, str]:
    error = _validate_v2(value)
    if error:
        return False, error
    if not verify_integrity(value):
        return False, "receipt integrity mismatch"

    actual_class, actual_reasons = classify_failure(value)
    declared = value["failure"]
    if declared["class"] != actual_class or declared["reasons"] != actual_reasons:
        return False, "failure classification mismatch"
    expected_passed = actual_class == "none"
    if value["verification"]["passed"] is not expected_passed:
        return False, "verification verdict mismatch"
    return (True, "verified") if expected_passed else (False, actual_class)
