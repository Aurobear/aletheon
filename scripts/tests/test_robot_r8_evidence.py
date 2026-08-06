#!/usr/bin/env python3

import argparse
import importlib.util
import json
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/libexec/aletheon/robot_r8_evidence.py"
SPEC = importlib.util.spec_from_file_location("robot_r8_evidence", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
R8 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(R8)


def report(settlement="completed", safe_stop=None):
    decision = "matched" if settlement == "completed" else "unsafe"
    value = {
        "format_version": 1,
        "episode_id": f"episode-{settlement}",
        "goal": "stand for three seconds",
        "device": "kuavo-mujoco-01",
        "sim_scene_version": "kuavo-mujoco/default-v40",
        "aletheon_commit": "abc123",
        "bridge_protocol_digest": "bridge-digest",
        "skill_descriptor_digest": "skill-digest",
        "policy_provenance": {
            "provider": "policy-gateway",
            "model": "openvla",
            "version": "2026-08",
            "protocol_version": "1.0",
            "digest": "model-digest",
        },
        "failures": [] if settlement == "completed" else [{"class": "unsafe", "detail": "fixture"}],
    }
    if safe_stop is not None:
        value["safe_stop"] = safe_stop
    value.update(
        {
            "selected_frames": [],
            "before_sequence": 10,
            "after_sequence": 11,
            "verified_sequence": 12,
            "settlement": settlement,
            "attempts": [
                {
                    "attempt": 1,
                    "attempt_id": "attempt-1",
                    "operation_id": "operation-1",
                    "request": {
                        "skill": "kuavo.stance",
                        "device": "kuavo-mujoco-01",
                        "parameters": {},
                    },
                    "expected": {
                        "predicate": {"kind": "equals", "path": "mode", "value": "stance"},
                        "freshness_ms": 500,
                        "stable_window_ms": 3_000,
                        "timeout_ms": 5_000,
                    },
                    "result_outcome": "Succeeded",
                    "verification_decision": {"decision": decision},
                    "verification_observed_paths": ["mode"],
                    "verification_reasons": [],
                    "retry_reason": None,
                    "before_sequence": 10,
                    "after_sequence": 11,
                    "verified_sequence": 12,
                    "evidence_refs": [],
                }
            ],
            "artifacts": [],
        }
    )
    return value


def arguments(settlement="completed", require_safe_stop=False):
    return argparse.Namespace(
        expected_device="kuavo-mujoco-01",
        expected_settlement=settlement,
        minimum_stable_window_ms=3_000,
        require_safe_stop=require_safe_stop,
        expected_safe_stop_outcome="succeeded",
        expected_scene="kuavo-mujoco/default-v40",
        expected_bridge_digest="bridge-digest",
        expected_skill_descriptor_digest="skill-digest",
        expected_policy_provider="policy-gateway",
        expected_policy_model="openvla",
        expected_policy_version="2026-08",
        expected_policy_protocol="1.0",
        expected_policy_digest="model-digest",
        expected_skill=None,
        expected_parameters=None,
    )


def database(path: Path, value):
    digest = R8._report_digest(value)
    receipt = {
        "report": value,
        "report_sha256": digest,
        "settled_at_unix_ms": 1_000,
    }
    connection = sqlite3.connect(path)
    connection.executescript(
        """
        CREATE TABLE episode_reports(
          episode_id TEXT PRIMARY KEY, report_json TEXT NOT NULL,
          report_sha256 TEXT NOT NULL, settlement TEXT NOT NULL,
          settled_at_unix_ms INTEGER NOT NULL
        );
        CREATE TABLE episodes(
          episode_id TEXT NOT NULL, attempt INTEGER NOT NULL, operation_id TEXT,
          request_json TEXT
        );
        CREATE TABLE episode_states(episode_id TEXT PRIMARY KEY, status TEXT NOT NULL);
        """
    )
    connection.execute(
        "INSERT INTO episode_reports VALUES (?, ?, ?, ?, ?)",
        (
            value["episode_id"],
            json.dumps(receipt, ensure_ascii=False, separators=(",", ":")),
            digest,
            value["settlement"],
            1_000,
        ),
    )
    connection.execute(
        "INSERT INTO episodes VALUES (?, 1, 'operation-1', ?)",
        (
            value["episode_id"],
            json.dumps(value["attempts"][0]["request"], ensure_ascii=False, separators=(",", ":")),
        ),
    )
    connection.execute(
        "INSERT INTO episode_states VALUES (?, ?)",
        (value["episode_id"], value["settlement"]),
    )
    connection.commit()
    connection.close()
    return digest


class RobotR8EvidenceTest(unittest.TestCase):
    def test_direct_checker_emits_bounded_receipt(self):
        value = report()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report_path = root / "report.json"
            database_path = root / "robot-episodes.db"
            output_path = root / "receipt.json"
            report_path.write_text(
                json.dumps(value, ensure_ascii=False, separators=(",", ":")),
                encoding="utf-8",
            )
            expected_digest = database(database_path, value)
            completed = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--report",
                    str(report_path),
                    "--database",
                    str(database_path),
                    "--expected-device",
                    "kuavo-mujoco-01",
                    "--expected-settlement",
                    "completed",
                    "--expected-scene",
                    "kuavo-mujoco/default-v40",
                    "--output",
                    str(output_path),
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            receipt = json.loads(completed.stdout)
            # Direct checker success is bounded report/SQLite evidence only.
            # Full R8 acceptance additionally runs the installed-runtime gate
            # through `aletheon.sh acceptance robot-r8`.
            self.assertEqual(receipt["status"], "REPORT_EVIDENCE_PASS")
            self.assertEqual(receipt["report_sha256"], expected_digest)
            self.assertEqual(json.loads(output_path.read_text()), receipt)

    def test_completed_report_matches_read_only_durable_receipt(self):
        value = report()
        summary = R8.validate_report(value, arguments())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "robot-episodes.db"
            expected = database(path, value)
            self.assertEqual(R8.validate_database(path, value, summary), expected)

    def test_negative_report_requires_observed_safe_stop_receipt(self):
        value = report(
            "failed",
            {
                "attempted_after_attempt": 1,
                "trigger": "unsafe",
                "outcome": "succeeded",
            },
        )
        summary = R8.validate_report(value, arguments("failed", True))
        self.assertEqual(summary["safe_stop"]["outcome"], "succeeded")
        value.pop("safe_stop")
        with self.assertRaisesRegex(R8.EvidenceError, "no typed safe_stop receipt"):
            R8.validate_report(value, arguments("failed", True))

        value = report(
            "failed",
            {
                "attempted_after_attempt": 0,
                "trigger": "unsafe",
                "outcome": "succeeded",
            },
        )
        with self.assertRaisesRegex(R8.EvidenceError, "final durable attempt"):
            R8.validate_report(value, arguments("failed", True))

    def test_failed_dispatch_allows_no_post_action_verification(self):
        value = report(
            "failed",
            {
                "attempted_after_attempt": 1,
                "trigger": "unsafe",
                "outcome": "succeeded",
            },
        )
        attempt = value["attempts"][0]
        attempt["expected"]["stable_window_ms"] = 500
        attempt["result_outcome"] = "Failed { reason: \"ownership rejected\" }"
        attempt["verification_decision"] = None
        attempt["verification_observed_paths"] = []
        attempt["after_sequence"] = None
        attempt["verified_sequence"] = None
        value["after_sequence"] = None
        value["verified_sequence"] = None

        summary = R8.validate_report(value, arguments("failed", True))
        self.assertEqual(summary["operation_ids"], ["operation-1"])

    def test_pre_execution_fallback_allows_zero_attempt_safe_stop(self):
        value = report(
            "failed",
            {
                "attempted_after_attempt": 0,
                "trigger": "proposal_rejected",
                "outcome": "succeeded",
            },
        )
        value["failures"] = [
            {
                "class": "proposal_rejected",
                "detail": "policy selected a typed safety fallback",
            }
        ]
        value["attempts"] = []
        value["before_sequence"] = None
        value["after_sequence"] = None
        value["verified_sequence"] = None

        summary = R8.validate_report(value, arguments("failed", True))
        self.assertEqual(summary["attempt_count"], 0)
        self.assertEqual(summary["operation_ids"], [])
        self.assertEqual(summary["governed_requests"], [])

    def test_pre_execution_fallback_has_zero_attempts_and_safe_stop(self):
        value = report(
            "failed",
            {
                "attempted_after_attempt": 0,
                "trigger": "proposal_rejected",
                "outcome": "succeeded",
            },
        )
        value["failures"] = [
            {"class": "proposal_rejected", "detail": "typed safety fallback"}
        ]
        value["attempts"] = []
        value["before_sequence"] = None
        value["after_sequence"] = None
        value["verified_sequence"] = None
        summary = R8.validate_report(value, arguments("failed", True))
        self.assertEqual(summary["attempt_count"], 0)
        self.assertEqual(summary["operation_ids"], [])

    def test_rejects_legacy_string_verification_decision(self):
        value = report()
        value["attempts"][0]["verification_decision"] = "matched"
        with self.assertRaisesRegex(R8.EvidenceError, "typed object"):
            R8.validate_report(value, arguments())

    def test_rejects_weak_or_unbound_attempt_evidence(self):
        value = report()
        value["attempts"][0]["operation_id"] = None
        with self.assertRaisesRegex(R8.EvidenceError, "operation_id"):
            R8.validate_report(value, arguments())

        value = report()
        value["attempts"][0].pop("request")
        with self.assertRaisesRegex(R8.EvidenceError, "request is absent"):
            R8.validate_report(value, arguments())

        value = report()
        value["attempts"][0]["expected"]["stable_window_ms"] = 2_999
        with self.assertRaisesRegex(R8.EvidenceError, "stable window"):
            R8.validate_report(value, arguments())

    def test_exact_governed_skill_and_parameters_are_checked(self):
        value = report()
        args = arguments()
        args.expected_skill = "kuavo.stance"
        args.expected_parameters = {}
        summary = R8.validate_report(value, args)
        self.assertEqual(summary["governed_requests"][0]["skill"], "kuavo.stance")

        args.expected_skill = "kuavo.move_base_timed"
        with self.assertRaisesRegex(R8.EvidenceError, "governed skill mismatch"):
            R8.validate_report(value, args)

    def test_rejects_database_receipt_drift(self):
        value = report()
        summary = R8.validate_report(value, arguments())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "robot-episodes.db"
            database(path, value)
            connection = sqlite3.connect(path)
            connection.execute(
                "UPDATE episode_reports SET report_sha256 = 'tampered'"
            )
            connection.commit()
            connection.close()
            with self.assertRaisesRegex(R8.EvidenceError, "digest"):
                R8.validate_database(path, value, summary)


if __name__ == "__main__":
    unittest.main()
