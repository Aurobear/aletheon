#!/usr/bin/env python3
"""Receipt contract and replay tests for the coding benchmark."""
from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent


def load_module(name: str, path: pathlib.Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


receipt = load_module("receipt", HERE / "harness/receipt.py")
replay = load_module("replay", HERE / "harness/replay.py")


class ReplayTest(unittest.TestCase):
    def base_v2(self) -> dict:
        value = {
            "schema_version": 2,
            "task_schema_version": 1,
            "task_id": "rust_bugfix",
            "category": "behavioral_bugfix",
            "binary": {
                "path": "/tmp/aletheon",
                "sha256": "sha256:" + "a" * 64,
            },
            "operation_id": "op",
            "observed_stop": "completed",
            "observed_terminal": "verified",
            "expected_terminal": "verified",
            "execution": {
                "argv": ["/tmp/aletheon", "exec"],
                "exit_code": 0,
                "timed_out": False,
                "elapsed_ms": 10,
                "stdout": "{}",
                "stderr": "",
                "stdout_digest": "sha256:" + "c" * 64,
                "stderr_digest": "sha256:" + "d" * 64,
                "stdout_truncated": False,
                "stderr_truncated": False,
                "json_valid": True,
                "terminal_snapshot": True,
                "reported_success": True,
                "process_group_reaped": True,
                "infrastructure_error": None,
            },
            "workspace": {
                "base_tree_digest": "sha256:" + "1" * 64,
                "diff": "diff --git a/src/lib.rs b/src/lib.rs\n",
                "diff_digest": "sha256:" + "b" * 64,
                "changed_files": ["src/lib.rs"],
                "forbidden_paths_unchanged": True,
                "required_scope_satisfied": True,
                "dirty_patch_digest": None,
                "dirty_patch_preserved": True,
            },
            "acceptance": [
                {
                    "argv": ["python3", "-c", "pass"],
                    "exit_code": 0,
                    "timed_out": False,
                    "elapsed_ms": 1,
                    "stdout": "",
                    "stderr": "",
                    "stdout_digest": "sha256:" + "e" * 64,
                    "stderr_digest": "sha256:" + "f" * 64,
                    "stdout_truncated": False,
                    "stderr_truncated": False,
                    "process_group_reaped": True,
                }
            ],
            "evidence": [
                {
                    "operation_id": "op",
                    "kind": "terminal_snapshot",
                    "observed_stop": "completed",
                },
                {
                    "operation_id": "op",
                    "kind": "acceptance_command",
                    "exit_code": 0,
                }
            ],
            "resources": {"passed": True, "checks": []},
            "metrics": {
                "iterations": 1,
                "tool_calls": 2,
                "tool_errors": 0,
                "inference_rounds": None,
                "provider_retries": 0,
                "active_context_tokens": None,
                "elapsed_ms": 10,
            },
            "failure": {"class": "none", "reasons": []},
            "verification": {"passed": True},
        }
        return receipt.seal(value)

    def replay_value(self, value: dict) -> tuple[bool, str]:
        with tempfile.NamedTemporaryFile("w", delete=False) as stream:
            json.dump(value, stream)
            path = pathlib.Path(stream.name)
        try:
            return replay.verify(path)
        finally:
            path.unlink()

    def classified(self, mutate) -> tuple[str, list[str]]:
        value = self.base_v2()
        mutate(value)
        return receipt.classify_failure(value)

    def test_v2_valid_receipt_replays(self):
        self.assertEqual(self.replay_value(self.base_v2()), (True, "verified"))

    def test_canonical_sealing_is_stable_and_tampering_fails(self):
        value = self.base_v2()
        original = value["integrity_sha256"]
        self.assertEqual(receipt.seal(copy.deepcopy(value))["integrity_sha256"], original)
        value["workspace"]["diff"] = "tampered"
        self.assertFalse(self.replay_value(value)[0])

    def test_v1_receipts_remain_replayable(self):
        value = {
            "schema_version": 1,
            "operation_id": "op",
            "workspace_diff": "diff",
            "evidence": [
                {
                    "operation_id": "op",
                    "kind": "acceptance_command",
                    "exit_code": 0,
                }
            ],
            "acceptance": [{"exit_code": 0, "timed_out": False}],
            "verification": {"passed": True},
            "terminal_status": "verified",
        }
        value["integrity_sha256"] = receipt.digest(
            json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
        )
        self.assertEqual(self.replay_value(value), (True, "verified"))

    def test_v2_required_and_unknown_fields_fail_closed(self):
        missing = self.base_v2()
        del missing["metrics"]
        receipt.seal(missing)
        self.assertFalse(self.replay_value(missing)[0])

        unknown = self.base_v2()
        unknown["derived_context_percent"] = 1
        receipt.seal(unknown)
        self.assertFalse(self.replay_value(unknown)[0])

    def test_failure_precedence_is_mutually_exclusive(self):
        value = self.base_v2()
        value["execution"]["infrastructure_error"] = "binary_unavailable"
        value["execution"]["timed_out"] = True
        value["workspace"]["forbidden_paths_unchanged"] = False
        value["observed_stop"] = "failed"
        value["acceptance"][0]["exit_code"] = 1
        failure_class, reasons = receipt.classify_failure(value)
        self.assertEqual(failure_class, "infrastructure_failure")
        self.assertEqual(reasons, ["binary_unavailable"])

        self.assertEqual(
            self.classified(lambda x: x["execution"].update(timed_out=True))[0],
            "timeout_or_cancellation",
        )
        self.assertEqual(
            self.classified(
                lambda x: x["workspace"].update(forbidden_paths_unchanged=False)
            )[0],
            "policy_scope_failure",
        )
        self.assertEqual(
            self.classified(lambda x: x.update(observed_stop="failed"))[0],
            "execution_failure",
        )
        self.assertEqual(
            self.classified(lambda x: x["acceptance"][0].update(exit_code=1))[0],
            "verification_failure",
        )

    def test_operation_and_evidence_must_correlate(self):
        missing_operation = self.base_v2()
        missing_operation["operation_id"] = ""
        klass, reasons = receipt.classify_failure(missing_operation)
        self.assertEqual(klass, "infrastructure_failure")
        self.assertIn("operation_id_missing", reasons)

        wrong_evidence = self.base_v2()
        wrong_evidence["evidence"][0]["operation_id"] = "another-op"
        klass, reasons = receipt.classify_failure(wrong_evidence)
        self.assertEqual(klass, "verification_failure")
        self.assertIn("operation_evidence_mismatch", reasons)

    def test_declared_false_success_is_rejected(self):
        value = self.base_v2()
        value["acceptance"][0]["exit_code"] = 1
        receipt.seal(value)
        ok, message = self.replay_value(value)
        self.assertFalse(ok)
        self.assertIn("failure classification mismatch", message)

        contradictory = self.base_v2()
        contradictory["observed_stop"] = "blocked"
        receipt.seal(contradictory)
        ok, message = self.replay_value(contradictory)
        self.assertFalse(ok)
        self.assertIn("contradicts authoritative stop", message)

        missing_success = self.base_v2()
        missing_success["execution"]["reported_success"] = None
        failure_class, reasons = receipt.classify_failure(missing_success)
        self.assertEqual(failure_class, "execution_failure")
        self.assertIn("client_reported_failure", reasons)

    def test_missing_metrics_remain_null(self):
        value = self.base_v2()
        self.assertIsNone(value["metrics"]["inference_rounds"])
        self.assertIsNone(value["metrics"]["active_context_tokens"])
        self.assertEqual(receipt.verify_receipt(value), (True, "verified"))

        derived = self.base_v2()
        derived["metrics"]["active_context_tokens"] = "derived:42%"
        receipt.seal(derived)
        self.assertFalse(self.replay_value(derived)[0])

    def test_expected_non_success_terminal_is_a_valid_benchmark_outcome(self):
        value = self.base_v2()
        value.update(
            observed_stop="blocked",
            observed_terminal="budget_exhausted",
            expected_terminal="budget_exhausted",
        )
        value["execution"]["exit_code"] = 1
        value["evidence"] = [
            {"operation_id": "op", "kind": "terminal_snapshot", "exit_code": 0},
            {"operation_id": "op", "kind": "acceptance_command", "exit_code": 0},
        ]
        value["metrics"]["iterations"] = 1
        receipt.seal(value)
        self.assertEqual(receipt.classify_failure(value), ("none", []))
        self.assertEqual(self.replay_value(value), (True, "verified"))

    def test_recovered_tool_error_is_diagnostic_when_host_gates_pass(self):
        value = self.base_v2()
        value["metrics"]["tool_errors"] = 2
        receipt.seal(value)
        self.assertEqual(receipt.classify_failure(value), ("none", []))
        self.assertEqual(self.replay_value(value), (True, "verified"))

        acceptance_failed = self.base_v2()
        acceptance_failed["metrics"]["tool_errors"] = 2
        acceptance_failed["acceptance"][0]["exit_code"] = 1
        failure_class, reasons = receipt.classify_failure(acceptance_failed)
        self.assertEqual(failure_class, "verification_failure")
        self.assertIn("acceptance_failed", reasons)


if __name__ == "__main__":
    unittest.main()
