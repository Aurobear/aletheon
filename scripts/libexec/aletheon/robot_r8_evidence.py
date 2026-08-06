#!/usr/bin/env python3
"""Validate one R8 EpisodeReport against its durable SQLite receipt.

This checker is deliberately read-only with respect to the runtime and episode
database. It does not start, stop, restart, or configure Aletheon, Policy,
Bridge, ROS, or MuJoCo. The operator performs the authorized live scenario
first, captures the one-shot `/usr/bin/aletheon` output, and runs this command
after the required daemon-restart observation.

Direct invocation proves report/SQLite evidence only. The public
`aletheon.sh acceptance robot-r8` entrypoint first runs the canonical installed
runtime provenance/stability/official-socket gate; neither half alone is full
R8 acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
from pathlib import Path
from typing import Any


class EvidenceError(RuntimeError):
    """The supplied runtime evidence does not prove the R8 contract."""


def _reject_json_constant(value: str) -> None:
    raise EvidenceError(f"non-finite JSON value is not accepted: {value}")


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"), parse_constant=_reject_json_constant
        )
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"read JSON evidence {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"JSON evidence is not an object: {path}")
    return value


def _require_string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise EvidenceError(f"{field} must be a non-empty string")
    return value


def _require_int(value: Any, field: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise EvidenceError(f"{field} must be a non-negative integer")
    return value


def _expect(actual: str, expected: str | None, field: str) -> None:
    if expected is not None and actual != expected:
        raise EvidenceError(f"{field} mismatch: expected={expected!r} actual={actual!r}")


def _report_digest(report: dict[str, Any]) -> str:
    # serde_json serializes struct fields in declaration order. Python preserves
    # object insertion order while parsing the stored receipt, so compact UTF-8
    # encoding reconstructs the exact bytes bound by SettledEpisodeReport.
    encoded = json.dumps(
        report,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def validate_report(report: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    if report.get("format_version") != 1:
        raise EvidenceError("R8 acceptance requires EpisodeReport format_version 1")
    episode_id = _require_string(report.get("episode_id"), "report.episode_id")
    device = _require_string(report.get("device"), "report.device")
    _expect(device, args.expected_device, "report.device")

    scene = _require_string(report.get("sim_scene_version"), "report.sim_scene_version")
    bridge_digest = _require_string(
        report.get("bridge_protocol_digest"), "report.bridge_protocol_digest"
    )
    descriptor_digest = _require_string(
        report.get("skill_descriptor_digest"), "report.skill_descriptor_digest"
    )
    _expect(scene, args.expected_scene, "report.sim_scene_version")
    _expect(bridge_digest, args.expected_bridge_digest, "report.bridge_protocol_digest")
    _expect(
        descriptor_digest,
        args.expected_skill_descriptor_digest,
        "report.skill_descriptor_digest",
    )

    policy = report.get("policy_provenance")
    if not isinstance(policy, dict):
        raise EvidenceError("report.policy_provenance must be a typed object")
    policy_facts = {
        field: _require_string(policy.get(field), f"report.policy_provenance.{field}")
        for field in ("provider", "model", "version", "protocol_version", "digest")
    }
    for field, expected in (
        ("provider", args.expected_policy_provider),
        ("model", args.expected_policy_model),
        ("version", args.expected_policy_version),
        ("protocol_version", args.expected_policy_protocol),
        ("digest", args.expected_policy_digest),
    ):
        _expect(policy_facts[field], expected, f"report.policy_provenance.{field}")

    settlement = _require_string(report.get("settlement"), "report.settlement")
    _expect(settlement, args.expected_settlement, "report.settlement")
    attempts = report.get("attempts")
    if not isinstance(attempts, list):
        raise EvidenceError("report.attempts must be a list")
    if not attempts and (not args.require_safe_stop or settlement == "completed"):
        raise EvidenceError(
            "zero-attempt R8 evidence is valid only for a failed pre-execution safe-stop path"
        )

    operation_ids: list[str] = []
    governed_requests: list[dict[str, Any]] = []
    verification_decisions: list[str | None] = []
    previous_verified = -1
    for index, attempt in enumerate(attempts, start=1):
        if not isinstance(attempt, dict):
            raise EvidenceError(f"report.attempts[{index - 1}] is not an object")
        if attempt.get("attempt") != index:
            raise EvidenceError("report attempts are not contiguous from attempt 1")
        _require_string(attempt.get("attempt_id"), f"report.attempts[{index - 1}].attempt_id")
        operation_id = _require_string(
            attempt.get("operation_id"),
            f"report.attempts[{index - 1}].operation_id",
        )
        if operation_id in operation_ids:
            raise EvidenceError(f"duplicate operation_id in report: {operation_id}")
        operation_ids.append(operation_id)

        request = attempt.get("request")
        if not isinstance(request, dict):
            raise EvidenceError(f"report.attempts[{index - 1}].request is absent")
        request_skill = _require_string(
            request.get("skill"), f"report.attempts[{index - 1}].request.skill"
        )
        request_device = _require_string(
            request.get("device"), f"report.attempts[{index - 1}].request.device"
        )
        if request_device != device:
            raise EvidenceError(f"attempt {index} request device differs from report device")
        parameters = request.get("parameters")
        if not isinstance(parameters, dict):
            raise EvidenceError(f"attempt {index} request parameters are not an object")
        governed_request = {
            "skill": request_skill,
            "device": request_device,
            "parameters": parameters,
        }
        governed_requests.append(governed_request)

        expected = attempt.get("expected")
        if not isinstance(expected, dict) or not isinstance(expected.get("predicate"), dict):
            raise EvidenceError(f"report.attempts[{index - 1}].expected.predicate is absent")
        _require_string(
            expected["predicate"].get("kind"),
            f"report.attempts[{index - 1}].expected.predicate.kind",
        )
        _require_int(
            expected.get("stable_window_ms"),
            f"report.attempts[{index - 1}].expected.stable_window_ms",
        )

        before = _require_int(
            attempt.get("before_sequence"),
            f"report.attempts[{index - 1}].before_sequence",
        )
        decision_object = attempt.get("verification_decision")
        observed = attempt.get("verification_observed_paths")
        if not isinstance(observed, list):
            raise EvidenceError(f"attempt {index} verification paths are not a list")
        for path_index, path in enumerate(observed):
            _require_string(path, f"attempt {index} observed path {path_index}")

        if decision_object is None:
            # A provider may reject an operation before it produces a post-action
            # observation. That is valid only for the explicit negative R8 lane:
            # the attempt must have failed, remain unverified, and be followed by
            # a typed terminal safe-stop receipt validated below.
            outcome = _require_string(
                attempt.get("result_outcome"),
                f"report.attempts[{index - 1}].result_outcome",
            )
            if (
                not args.require_safe_stop
                or settlement == "completed"
                or not outcome.startswith("Failed")
                or attempt.get("after_sequence") is not None
                or attempt.get("verified_sequence") is not None
                or observed
            ):
                raise EvidenceError(
                    f"attempt {index} lacks typed verification outside a "
                    "failed pre-verification safe-stop path"
                )
            verification_decisions.append(None)
            continue

        if not isinstance(decision_object, dict):
            raise EvidenceError(
                f"report.attempts[{index - 1}].verification_decision "
                "must be a typed object"
            )
        decision = _require_string(
            decision_object.get("decision"),
            f"report.attempts[{index - 1}].verification_decision.decision",
        )
        if decision not in {
            "matched",
            "retryable_mismatch",
            "replannable_mismatch",
            "unsafe",
            "unknown",
        }:
            raise EvidenceError(f"attempt {index} has unknown verification decision {decision!r}")
        verification_decisions.append(decision)
        after = _require_int(
            attempt.get("after_sequence"),
            f"report.attempts[{index - 1}].after_sequence",
        )
        verified = _require_int(
            attempt.get("verified_sequence"),
            f"report.attempts[{index - 1}].verified_sequence",
        )
        if not before <= after <= verified or verified < previous_verified:
            raise EvidenceError(f"attempt {index} sequence order is invalid")
        previous_verified = verified
        if not observed:
            raise EvidenceError(f"attempt {index} has no observed verification paths")

    if attempts:
        if report.get("before_sequence") != attempts[0]["before_sequence"]:
            raise EvidenceError("report.before_sequence does not match first attempt")
        if report.get("after_sequence") != attempts[-1]["after_sequence"]:
            raise EvidenceError("report.after_sequence does not match final attempt")
        if report.get("verified_sequence") != attempts[-1]["verified_sequence"]:
            raise EvidenceError("report.verified_sequence does not match final attempt")
    elif any(
        report.get(field) is not None
        for field in ("before_sequence", "after_sequence", "verified_sequence")
    ):
        raise EvidenceError("zero-attempt report contains fabricated sequence summaries")

    safe_stop = report.get("safe_stop")
    if args.require_safe_stop:
        if not isinstance(safe_stop, dict):
            raise EvidenceError("negative R8 scenario has no typed safe_stop receipt")
        if safe_stop.get("outcome") != args.expected_safe_stop_outcome:
            raise EvidenceError(
                "safe_stop outcome mismatch: "
                f"expected={args.expected_safe_stop_outcome!r} "
                f"actual={safe_stop.get('outcome')!r}"
            )
        after_attempt = _require_int(
            safe_stop.get("attempted_after_attempt"),
            "report.safe_stop.attempted_after_attempt",
        )
        if after_attempt != len(attempts):
            raise EvidenceError("safe_stop receipt is not bound to the final durable attempt")
        trigger = safe_stop.get("trigger")
        if trigger is not None:
            _require_string(trigger, "report.safe_stop.trigger")
            failures = report.get("failures")
            if not isinstance(failures, list) or not any(
                isinstance(failure, dict) and failure.get("class") == trigger
                for failure in failures
            ):
                raise EvidenceError("safe_stop trigger is absent from typed failure history")
        failures = report.get("failures")
        has_safe_stop_failure = isinstance(failures, list) and any(
            isinstance(failure, dict) and failure.get("class") == "safe_stop_failure"
            for failure in failures
        )
        if (safe_stop.get("outcome") == "failed") != has_safe_stop_failure:
            raise EvidenceError("safe_stop outcome conflicts with typed failure history")
        if settlement == "completed":
            raise EvidenceError("completed episode cannot prove a negative safe-stop scenario")
    elif settlement == "completed" and safe_stop is not None:
        raise EvidenceError("completed episode unexpectedly contains safe_stop evidence")

    artifacts = report.get("artifacts")
    if not isinstance(artifacts, list):
        raise EvidenceError("report.artifacts must be a list")
    for index, artifact in enumerate(artifacts):
        if not isinstance(artifact, dict):
            raise EvidenceError(f"report.artifacts[{index}] is not an object")
        uri = _require_string(artifact.get("uri"), f"report.artifacts[{index}].uri")
        if uri.lower().startswith(("data:", "inline:")):
            raise EvidenceError(f"report.artifacts[{index}] contains inline content")

    if settlement == "completed":
        stable_window_ms = attempts[-1]["expected"]["stable_window_ms"]
        if stable_window_ms < args.minimum_stable_window_ms:
            raise EvidenceError(
                f"final stable window {stable_window_ms}ms is below required "
                f"{args.minimum_stable_window_ms}ms"
            )
    if settlement == "completed" and verification_decisions[-1] != "matched":
        raise EvidenceError("completed episode final verification is not matched")
    expected_skill = getattr(args, "expected_skill", None)
    if expected_skill is not None and not governed_requests:
        raise EvidenceError("expected governed skill but episode has no executed attempt")
    if expected_skill is not None and governed_requests[-1]["skill"] != expected_skill:
        raise EvidenceError(
            "final governed skill mismatch: "
            f"expected={expected_skill!r} actual={governed_requests[-1]['skill']!r}"
        )
    expected_parameters = getattr(args, "expected_parameters", None)
    if expected_parameters is not None and not governed_requests:
        raise EvidenceError("expected governed parameters but episode has no executed attempt")
    if expected_parameters is not None and governed_requests[-1]["parameters"] != expected_parameters:
        raise EvidenceError(
            "final governed parameters mismatch: "
            f"expected={expected_parameters!r} "
            f"actual={governed_requests[-1]['parameters']!r}"
        )

    return {
        "episode_id": episode_id,
        "settlement": settlement,
        "device": device,
        "scene": scene,
        "bridge_protocol_digest": bridge_digest,
        "skill_descriptor_digest": descriptor_digest,
        "policy": policy_facts,
        "operation_ids": operation_ids,
        "governed_requests": governed_requests,
        "attempt_count": len(attempts),
        "safe_stop": safe_stop,
    }


def validate_database(
    database: Path, report: dict[str, Any], summary: dict[str, Any]
) -> str:
    if not database.is_file():
        raise EvidenceError(f"episode database is unavailable: {database}")
    uri = f"{database.resolve().as_uri()}?mode=ro"
    try:
        connection = sqlite3.connect(uri, uri=True)
        connection.execute("PRAGMA query_only=ON")
        row = connection.execute(
            "SELECT report_json, report_sha256, settlement, settled_at_unix_ms "
            "FROM episode_reports WHERE episode_id = ?",
            (summary["episode_id"],),
        ).fetchone()
        if row is None:
            raise EvidenceError("durable episode report was not found after restart")
        try:
            receipt = json.loads(row[0], parse_constant=_reject_json_constant)
        except (TypeError, json.JSONDecodeError) as error:
            raise EvidenceError(f"durable settled report is not valid JSON: {error}") from error
        if not isinstance(receipt, dict) or receipt.get("report") != report:
            raise EvidenceError("durable settled report differs from official-client output")
        actual_digest = _report_digest(receipt["report"])
        receipt_digest = _require_string(
            receipt.get("report_sha256"), "settled_report.report_sha256"
        )
        if actual_digest != receipt_digest or row[1] != receipt_digest:
            raise EvidenceError("durable settled report digest does not match report bytes")
        if row[2] != summary["settlement"]:
            raise EvidenceError("durable report settlement column differs from report")
        if receipt.get("settled_at_unix_ms") != row[3]:
            raise EvidenceError("durable report settlement timestamp column differs from receipt")

        durable_attempts = connection.execute(
            "SELECT attempt, operation_id, request_json FROM episodes "
            "WHERE episode_id = ? ORDER BY attempt ASC",
            (summary["episode_id"],),
        ).fetchall()
        expected_attempts = [
            (index, operation_id, json.dumps(request, ensure_ascii=False, separators=(",", ":")))
            for index, (operation_id, request) in enumerate(
                zip(summary["operation_ids"], summary["governed_requests"], strict=True),
                start=1,
            )
        ]
        if durable_attempts != expected_attempts:
            raise EvidenceError("durable attempt/operation order differs from EpisodeReport")
        state = connection.execute(
            "SELECT status FROM episode_states WHERE episode_id = ?",
            (summary["episode_id"],),
        ).fetchone()
        if state != (summary["settlement"],):
            raise EvidenceError("durable episode state differs from report settlement")
        integrity = connection.execute("PRAGMA integrity_check").fetchone()
        if integrity != ("ok",):
            raise EvidenceError(f"episode database integrity check failed: {integrity}")
        return receipt_digest
    except sqlite3.Error as error:
        raise EvidenceError(f"read episode database: {error}") from error
    finally:
        if "connection" in locals():
            connection.close()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--expected-device", required=True)
    parser.add_argument("--expected-settlement", choices=("completed", "failed", "cancelled"), required=True)
    parser.add_argument("--minimum-stable-window-ms", type=int, default=3_000)
    parser.add_argument("--require-safe-stop", action="store_true")
    parser.add_argument(
        "--expected-safe-stop-outcome",
        choices=("succeeded", "failed"),
        default="succeeded",
    )
    parser.add_argument("--expected-scene")
    parser.add_argument("--expected-bridge-digest")
    parser.add_argument("--expected-skill-descriptor-digest")
    parser.add_argument("--expected-policy-provider")
    parser.add_argument("--expected-policy-model")
    parser.add_argument("--expected-policy-version")
    parser.add_argument("--expected-policy-protocol")
    parser.add_argument("--expected-policy-digest")
    parser.add_argument("--expected-skill")
    parser.add_argument(
        "--expected-parameters-json",
        help="exact JSON object expected in the final governed SkillRequest",
    )
    parser.add_argument("--output", type=Path)
    return parser


def main() -> None:
    args = _parser().parse_args()
    if args.minimum_stable_window_ms < 0:
        raise SystemExit("--minimum-stable-window-ms must be non-negative")
    args.expected_parameters = None
    if args.expected_parameters_json is not None:
        try:
            args.expected_parameters = json.loads(
                args.expected_parameters_json, parse_constant=_reject_json_constant
            )
        except (json.JSONDecodeError, EvidenceError) as error:
            raise SystemExit(f"--expected-parameters-json is invalid: {error}") from error
        if not isinstance(args.expected_parameters, dict):
            raise SystemExit("--expected-parameters-json must decode to an object")
    try:
        report = _load_json(args.report)
        summary = validate_report(report, args)
        report_sha256 = validate_database(args.database, report, summary)
    except EvidenceError as error:
        raise SystemExit(f"R8 evidence rejected: {error}") from error

    receipt = {
        "schema_version": 1,
        "status": "REPORT_EVIDENCE_PASS",
        "episode_id": summary["episode_id"],
        "settlement": summary["settlement"],
        "report_sha256": report_sha256,
        "attempt_count": summary["attempt_count"],
        "operation_ids": summary["operation_ids"],
        "safe_stop": summary["safe_stop"],
        "runtime_facts": {
            "device": summary["device"],
            "scene": summary["scene"],
            "bridge_protocol_digest": summary["bridge_protocol_digest"],
            "skill_descriptor_digest": summary["skill_descriptor_digest"],
            "policy": summary["policy"],
        },
    }
    rendered = json.dumps(receipt, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_name(f".{args.output.name}.tmp")
        temporary.write_text(rendered + "\n", encoding="utf-8")
        temporary.replace(args.output)
    print(rendered)


if __name__ == "__main__":
    main()
