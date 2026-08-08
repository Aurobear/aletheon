#!/usr/bin/env python3
"""E1 Robot Engineering Suite Report Generator.

Produces per-task records and domain/risk/cost/failure-class slices.
Distinguishes fixture validation from real installed evaluation.

Validation:
- Rejects mixed modes (all records must share same mode).
- Rejects duplicate task IDs.
- Rejects malformed records (missing required keys, invalid types).
- model_acceptance true in fixture mode is rejected.
- model_acceptance_reported only true for validated real-evaluation, not merely `not fixture`.

Metric names consistent with runner: tool_calls_made, tool_errors, provider_retries,
elapsed_ms, iterations, completed_normally.
Tokens/cache/context/inference-rounds remain unavailable/null until later protocol work.
Averages divide by available_count, not total task count.
Tracks availability per metric, not globally.

Scoring dimensions: correctness, test_quality (unavailable), safety_boundary,
scope_architecture_consistency, first_attempt_success, elapsed,
token/cache cost (unavailable), grounded tool cost when reported, and
human_intervention (unavailable until authoritative evidence exists).
"""
import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any, Dict, List

# Real TurnMetrics fields from crates/fabric/src/types/turn.rs:84-92
REAL_METRICS = (
    "tool_calls_made", "tool_errors", "provider_retries",
    "elapsed_ms", "iterations", "completed_normally",
)
# Unavailable until protocol work authorizes them
UNAVAILABLE_METRICS = (
    "tokens_input", "tokens_output", "cache_hits", "active_context_tokens",
)
ALL_METRICS = REAL_METRICS + UNAVAILABLE_METRICS

_REQUIRED_CATEGORY_COUNTS = {
    "ros2_lifecycle_launch": 6,
    "containers_build": 4,
    "can_ethercat_serial": 5,
    "control_numerics": 5,
    "sensing_logs": 4,
    "safety_device_protocol": 3,
    "multi_file_engineering": 3,
}

SLICE_DIMS = ("domain", "risk", "cost", "failure_class")

REQUIRED_RECORD_KEYS = frozenset({
    "task_id", "category", "risk", "expected_failure_class",
    "permission", "mode", "model_acceptance",
    "cpu_seconds", "memory_mb", "max_open_files", "max_processes", "timeout",
})
# Upper bounds mirror catalog validation in runner.py
_RESOURCE_UPPER_BOUNDS = {
    "cpu_seconds": 60,
    "memory_mb": 4096,
    "max_open_files": 1024,
    "max_processes": 8192,
    "timeout": 3600,
}
_RESOURCE_FIELDS = ("cpu_seconds", "memory_mb", "max_open_files", "max_processes", "timeout")
VALID_MODES = frozenset({"fixture-validation", "real-evaluation"})
VALID_RECORD_PERMISSIONS = frozenset({
    "read_only", "device_action", "code_modification", "simulation",
})
VALID_RECORD_RISKS = frozenset({"low", "medium", "high"})

# Known scoring field → expected type (None means "unavailable" string is acceptable)
_VALID_SCORING_FIELDS = {
    "correctness": (bool, type(None)),
    "test_quality": (str,),          # "unavailable" only
    "safety_boundary": (bool, type(None)),
    "scope_architecture_consistency": (bool, type(None)),
    "first_attempt_success": (bool, type(None)),
    "elapsed_ms": (int, type(None)),
    "token_cost": (str,),            # "unavailable" only
    "cache_cost": (str,),            # "unavailable" only
    "tool_cost": (int, str),         # nonnegative int or "unavailable"
    "human_intervention": (str,),    # "unavailable" only
}
_VALID_SCORING_KEYS = frozenset(_VALID_SCORING_FIELDS.keys())

# Known metrics_available keys
_VALID_METRICS_AVAILABLE_KEYS = frozenset({
    "tool_calls_made", "tool_errors", "provider_retries",
    "elapsed_ms", "iterations", "completed_normally",
    "tokens_input", "tokens_output", "cache_hits", "active_context_tokens",
})
SCORING_KEYS = frozenset({
    "correctness", "test_quality", "safety_boundary",
    "scope_architecture_consistency", "first_attempt_success", "elapsed_ms",
    "token_cost", "cache_cost", "tool_cost", "human_intervention",
})


def _validate_records(records: List[dict]) -> List[str]:
    """Fail-closed per-record validation. Returns list of error strings.
    Every record is validated independently; an error in one record
    does NOT skip validation of later records."""
    errors: List[str] = []
    if not isinstance(records, list):
        return ["records must be a list"]
    if not records:
        return ["no records"]

    modes = set()
    seen_ids: set = set()
    for i, r in enumerate(records):
        if not isinstance(r, dict):
            errors.append(f"record {i}: not a dict, got {type(r).__name__}")
            continue

        record_errs: List[str] = []

        # Required keys
        for key in REQUIRED_RECORD_KEYS:
            if key not in r:
                record_errs.append(f"record {i}: missing key {key}")

        if record_errs:
            errors.extend(record_errs)
            continue

        tid = r.get("task_id", f"record_{i}")
        mode = r.get("mode", "unknown")

        # Accept only exact modes
        if mode not in VALID_MODES:
            errors.append(f"{tid}: invalid mode {mode!r}")
            continue

        modes.add(mode)

        # Require typed bool model_acceptance
        ma = r.get("model_acceptance")
        if not isinstance(ma, bool):
            errors.append(
                f"{tid}: model_acceptance must be bool, got {type(ma).__name__}"
            )

        # Validate task_id string
        if not isinstance(tid, str) or not tid:
            errors.append(f"record {i}: task_id must be nonempty string")

        # Validate category
        cat = r.get("category")
        if not isinstance(cat, str) or cat not in _REQUIRED_CATEGORY_COUNTS:
            errors.append(
                f"{tid}: category {cat!r} not in "
                f"{sorted(_REQUIRED_CATEGORY_COUNTS.keys())}"
            )

        # Validate risk
        risk = r.get("risk")
        if not isinstance(risk, str) or risk not in ("low", "medium", "high"):
            errors.append(f"{tid}: invalid risk {risk!r}")

        # Validate expected_failure_class
        efc = r.get("expected_failure_class")
        if not isinstance(efc, str) or not efc:
            errors.append(f"{tid}: expected_failure_class must be nonempty string")

        injected = r.get("injected_failures")
        if not isinstance(injected, list) or not injected or not all(
            isinstance(item, str) and item for item in injected
        ):
            errors.append(f"{tid}: injected_failures must be nonempty strings")

        # Validate permission
        perm = r.get("permission")
        if not isinstance(perm, str) or perm not in VALID_RECORD_PERMISSIONS:
            errors.append(f"{tid}: invalid permission {perm!r}")

        cost_tier = r.get("cost_tier")
        if cost_tier not in {"low", "high"}:
            errors.append(f"{tid}: invalid cost_tier {cost_tier!r}")
        else:
            # cost_tier must match the deterministic tier derived from cpu_seconds
            cpu = r.get("cpu_seconds", 0)
            if isinstance(cpu, int) and not isinstance(cpu, bool):
                expected_tier = "high" if cpu >= 8 else "low"
                if cost_tier != expected_tier:
                    errors.append(
                        f"{tid}: cost_tier {cost_tier!r} contradicts "
                        f"cpu_seconds {cpu} (expected {expected_tier!r})"
                    )

        # Validate resource fields: must be non-bool positive ints within upper bounds
        for res_key in _RESOURCE_FIELDS:
            val = r.get(res_key)
            if val is None:
                errors.append(f"{tid}: {res_key} is missing")
                continue
            if not isinstance(val, int) or isinstance(val, bool):
                errors.append(
                    f"{tid}: {res_key} must be int, "
                    f"got {type(val).__name__}: {val!r}"
                )
                continue
            if val <= 0:
                errors.append(f"{tid}: {res_key} must be > 0, got {val}")
            if val > _RESOURCE_UPPER_BOUNDS[res_key]:
                errors.append(
                    f"{tid}: {res_key} {val} exceeds max "
                    f"{_RESOURCE_UPPER_BOUNDS[res_key]}"
                )

        # Metric values are schema-typed before any aggregation.
        for metric in REAL_METRICS:
            value = r.get(metric)
            if metric == "completed_normally":
                valid = value is None or isinstance(value, bool)
            else:
                valid = (
                    value is None
                    or (
                        isinstance(value, int)
                        and not isinstance(value, bool)
                        and value >= 0
                    )
                )
            if not valid:
                errors.append(f"{tid}: invalid metric {metric}={value!r}")
        for metric in UNAVAILABLE_METRICS:
            if r.get(metric) is not None:
                errors.append(f"{tid}: unavailable metric {metric} must be null")

        # Validate metrics_available as an exact typed map consistent with values.
        ma_metrics = r.get("metrics_available")
        if not isinstance(ma_metrics, dict):
            errors.append(
                f"{tid}: metrics_available must be dict, "
                f"got {type(ma_metrics).__name__}"
            )
        elif set(ma_metrics) != set(ALL_METRICS):
            errors.append(f"{tid}: metrics_available keys do not match metrics")
        else:
            for metric, available in ma_metrics.items():
                if not isinstance(available, bool):
                    errors.append(
                        f"{tid}: metrics_available.{metric} must be bool"
                    )
                elif available != (r.get(metric) is not None):
                    errors.append(
                        f"{tid}: metrics_available.{metric} contradicts value"
                    )

        # Validate scoring is a typed dict for real-evaluation records
        if mode == "real-evaluation":
            if not isinstance(r.get("oracle_passed"), bool):
                errors.append(f"{tid}: oracle_passed must be bool")
            if not isinstance(r.get("failure_class"), str) or not r.get("failure_class"):
                errors.append(f"{tid}: failure_class must be nonempty string")
            reasons = r.get("failure_reasons")
            if not isinstance(reasons, list) or not all(
                isinstance(reason, str) for reason in reasons
            ):
                errors.append(f"{tid}: failure_reasons must be strings")

            scoring = r.get("scoring")
            if not isinstance(scoring, dict):
                errors.append(
                    f"{tid}: scoring must be dict in real-evaluation, "
                    f"got {type(scoring).__name__}"
                )
            elif set(scoring) != SCORING_KEYS:
                errors.append(f"{tid}: scoring keys do not match contract")
            else:
                for key in (
                    "correctness", "safety_boundary",
                    "scope_architecture_consistency", "first_attempt_success",
                ):
                    if scoring[key] is not None and not isinstance(scoring[key], bool):
                        errors.append(f"{tid}: scoring.{key} must be bool or null")
                elapsed = scoring["elapsed_ms"]
                if elapsed is not None and not (
                    isinstance(elapsed, int)
                    and not isinstance(elapsed, bool)
                    and elapsed >= 0
                ):
                    errors.append(f"{tid}: scoring.elapsed_ms must be nonnegative int or null")
                if scoring["test_quality"] != "unavailable":
                    errors.append(f"{tid}: scoring.test_quality must be unavailable")
                for key in ("token_cost", "cache_cost"):
                    if scoring[key] != "unavailable":
                        errors.append(f"{tid}: scoring.{key} must be unavailable")
                tool_cost = scoring["tool_cost"]
                if tool_cost != "unavailable" and not (
                    isinstance(tool_cost, int)
                    and not isinstance(tool_cost, bool)
                    and tool_cost >= 0
                ):
                    errors.append(f"{tid}: scoring.tool_cost is invalid")
                if scoring["human_intervention"] != "unavailable":
                    errors.append(
                        f"{tid}: scoring.human_intervention must be unavailable"
                    )
        else:
            for key in (
                "baseline_passed", "solution_passed", "solution_leaked",
                "baseline_has_structured_evidence", "oracle_infrastructure_error",
            ):
                if not isinstance(r.get(key), bool):
                    errors.append(f"{tid}: {key} must be bool in fixture-validation")

        # Reject model_acceptance true in fixture mode
        if mode == "fixture-validation" and ma is True:
            errors.append(
                f"{tid}: model_acceptance=true in fixture-validation mode"
            )

        # Reject duplicate task IDs
        if tid in seen_ids:
            errors.append(f"record {i}: duplicate task_id {tid}")
        seen_ids.add(tid)

    # Reject mixed modes
    if len(modes) > 1:
        errors.append(f"mixed modes: {modes}")

    return errors


def _empty_slice_entry() -> dict:
    item = {"count": 0, "pass_count": 0, "metric_counts": {}}
    for m in ALL_METRICS:
        item[f"{m}_sum"] = 0
        item[f"{m}_available_count"] = 0
        item[f"{m}_avg"] = None
    return item


def _update_slice_entry(item: dict, record: dict, mode: str) -> None:
    item["count"] += 1
    # pass_count is mode-specific:
    #   fixture-validation: solution_passed only (fixture integrity)
    #   real-evaluation: model_acceptance only
    if mode == "fixture-validation":
        passed = bool(record.get("solution_passed"))
    else:
        passed = bool(record.get("model_acceptance"))
    if passed:
        item["pass_count"] += 1
    for m in ALL_METRICS:
        val = record.get(m)
        # Guard: only accept valid numeric types (int, float, bool for completed_normally)
        if val is not None:
            if m == "completed_normally":
                if isinstance(val, bool):
                    item[f"{m}_sum"] += 1 if val else 0
                    item[f"{m}_available_count"] += 1
            elif isinstance(val, (int, float)) and not isinstance(val, bool):
                if val >= 0:
                    item[f"{m}_sum"] += val
                    item[f"{m}_available_count"] += 1


def _finalize_slice_entry(item: dict) -> None:
    for m in ALL_METRICS:
        count = item[f"{m}_available_count"]
        if count > 0:
            item[f"{m}_avg"] = item[f"{m}_sum"] / count
        else:
            item[f"{m}_avg"] = None
    item.pop("metric_counts", None)


def _cost_tier(record: dict) -> str:
    """Return the validated, explicit cost tier."""
    return record["cost_tier"]


def summarize(records: List[dict]) -> dict:
    """Summarize E1 suite results with domain/risk/cost/failure slices.

    Raises ValueError for invalid records (mixed modes, dup IDs, fixture acceptance).
    """
    validation_errors = _validate_records(records)
    if validation_errors:
        raise ValueError("Record validation failed: " + "; ".join(validation_errors))

    if not records:
        return {
            "total": 0,
            "mode": "unknown",
            "is_fixture_validation": True,
            "model_acceptance_reported": False,
            "slices": {dim: {} for dim in SLICE_DIMS},
            "per_task": [],
            "scoring_summary": {},
        }

    mode = records[0].get("mode", "unknown")
    is_fixture = mode == "fixture-validation"

    # model_acceptance_reported: a valid real-evaluation outcome was
    # produced and reported (even when every task failed).
    # True for real-evaluation mode, regardless of pass/fail count.
    model_acceptance_reported = not is_fixture

    slices: Dict[str, Dict[str, dict]] = {
        dim: defaultdict(_empty_slice_entry) for dim in SLICE_DIMS
    }

    per_task = []
    for r in records:
        tid = r.get("task_id", "unknown")
        cat = r.get("category", "unknown")
        risk = r.get("risk", "unknown")
        fail_class = r.get("expected_failure_class", "unknown")

        # Per-task immutable record
        task_record = {
            "task_id": tid,
            "category": cat,
            "risk": risk,
            "expected_failure_class": fail_class,
            "permission": r.get("permission", "unknown"),
            "injected_failures": r.get("injected_failures", []),
            "baseline_passed": r.get("baseline_passed"),
            "solution_passed": r.get("solution_passed"),
            "oracle_passed": r.get("oracle_passed"),
            "model_acceptance": r.get("model_acceptance", False),
            "failure_class": r.get("failure_class"),
            "failure_reasons": r.get("failure_reasons", []),
            "mode": mode,
            "solution_leaked": r.get("solution_leaked", False),
            "leaked_files": r.get("leaked_files", []),
            "baseline_has_structured_evidence": r.get(
                "baseline_has_structured_evidence"
            ),
            "cost_tier": _cost_tier(r),
        }
        for m in ALL_METRICS:
            task_record[m] = r.get(m)
        task_record["metrics_available"] = r.get("metrics_available", {})
        task_record["scoring"] = r.get("scoring", {})
        per_task.append(task_record)

        # Domain slice
        _update_slice_entry(slices["domain"][cat], r, mode)
        # Risk slice
        _update_slice_entry(slices["risk"][risk], r, mode)
        # Cost slice
        cost = _cost_tier(r)
        _update_slice_entry(slices["cost"][cost], r, mode)
        # Failure class slice
        _update_slice_entry(slices["failure_class"][fail_class], r, mode)

    for dim_slices in slices.values():
        for item in dim_slices.values():
            _finalize_slice_entry(item)

    result_slices = {dim: dict(d) for dim, d in slices.items()}

    # Scoring summary — honestly expose availability/count/average or rate
    # for grounded dimensions; explicitly mark ungrounded as unavailable.
    scoring_summary = {
        "correctness_available": False,
        "correctness_rate": None,
        "correctness_count": 0,
        "test_quality_available": False,
        "safety_boundary_available": False,
        "safety_boundary_rate": None,
        "safety_boundary_count": 0,
        "scope_consistency_available": False,
        "scope_consistency_rate": None,
        "scope_consistency_count": 0,
        "first_attempt_available": False,
        "first_attempt_rate": None,
        "first_attempt_count": 0,
        "elapsed_available": False,
        "elapsed_avg_ms": None,
        "elapsed_count": 0,
        "token_cost_available": False,
        "cache_cost_available": False,
        "tool_cost_available": False,
        "tool_calls_available": False,
        "tool_calls_avg": None,
        "tool_calls_count": 0,
        "human_intervention_available": False,
    }
    if not is_fixture:
        scores = [r.get("scoring", {}) for r in records
                  if isinstance(r.get("scoring"), dict)]
        correctness_vals = [s.get("correctness") for s in scores
                            if isinstance(s.get("correctness"), bool)]
        safety_vals = [s.get("safety_boundary") for s in scores
                       if isinstance(s.get("safety_boundary"), bool)]
        scope_vals = [s.get("scope_architecture_consistency") for s in scores
                      if isinstance(s.get("scope_architecture_consistency"), bool)]
        first_vals = [s.get("first_attempt_success") for s in scores
                      if isinstance(s.get("first_attempt_success"), bool)]
        elapsed_vals = [r.get("elapsed_ms") for r in records
                        if isinstance(r.get("elapsed_ms"), int)
                        and not isinstance(r.get("elapsed_ms"), bool)]
        tool_calls_vals = [r.get("tool_calls_made") for r in records
                           if isinstance(r.get("tool_calls_made"), int)
                           and not isinstance(r.get("tool_calls_made"), bool)]

        if correctness_vals:
            scoring_summary["correctness_available"] = True
            scoring_summary["correctness_rate"] = sum(correctness_vals) / len(correctness_vals)
            scoring_summary["correctness_count"] = len(correctness_vals)
        if safety_vals:
            scoring_summary["safety_boundary_available"] = True
            scoring_summary["safety_boundary_rate"] = sum(safety_vals) / len(safety_vals)
            scoring_summary["safety_boundary_count"] = len(safety_vals)
        if scope_vals:
            scoring_summary["scope_consistency_available"] = True
            scoring_summary["scope_consistency_rate"] = sum(scope_vals) / len(scope_vals)
            scoring_summary["scope_consistency_count"] = len(scope_vals)
        if first_vals:
            scoring_summary["first_attempt_available"] = True
            scoring_summary["first_attempt_rate"] = sum(first_vals) / len(first_vals)
            scoring_summary["first_attempt_count"] = len(first_vals)
        if elapsed_vals:
            scoring_summary["elapsed_available"] = True
            scoring_summary["elapsed_avg_ms"] = sum(elapsed_vals) / len(elapsed_vals)
            scoring_summary["elapsed_count"] = len(elapsed_vals)
        if tool_calls_vals:
            scoring_summary["tool_calls_available"] = True
            scoring_summary["tool_calls_avg"] = sum(tool_calls_vals) / len(tool_calls_vals)
            scoring_summary["tool_calls_count"] = len(tool_calls_vals)
            # When grounded tool_calls_made exists, tool_cost is available
            scoring_summary["tool_cost_available"] = True

    return {
        "total": len(records),
        "mode": mode,
        "is_fixture_validation": is_fixture,
        "model_acceptance_reported": model_acceptance_reported,
        "slices": result_slices,
        "per_task": per_task,
        "scoring_summary": scoring_summary,
    }


def main():
    ap = argparse.ArgumentParser(description="E1 Report Generator")
    ap.add_argument("--input", default="", help="Input JSON file (default: stdin)")
    args = ap.parse_args()
    if args.input:
        data = json.loads(Path(args.input).read_text(encoding="utf-8"))
    else:
        data = json.load(sys.stdin)
    try:
        result = summarize(data)
        print(json.dumps(result, indent=2, sort_keys=True))
    except ValueError as e:
        print(f"REPORT_ERROR: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
