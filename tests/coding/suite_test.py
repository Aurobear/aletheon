#!/usr/bin/env python3
"""Deterministic aggregation tests for coding benchmark suite receipts."""
from __future__ import annotations

import copy
import fcntl
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
HARNESS = HERE / "harness"
sys.path.insert(0, str(HARNESS))


def load_module(name: str, path: pathlib.Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


receipt = load_module("suite_receipt", HARNESS / "receipt.py")
suite = load_module("coding_suite", HARNESS / "suite.py")


def command(exit_code: int = 0) -> dict:
    return {
        "argv": ["python3", "-c", "pass"],
        "exit_code": exit_code,
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


def make_receipt(
    task_id: str,
    *,
    category: str = "behavioral_bugfix",
    expected: str = "verified",
    observed_stop: str = "completed",
    observed_terminal: str = "verified",
    operation_id: str = "op",
    metric: int | None = 10,
    failure_mutation=None,
) -> dict:
    value = {
        "schema_version": 2,
        "task_schema_version": 1,
        "task_id": task_id,
        "category": category,
        "binary": {"path": "/usr/bin/aletheon", "sha256": "sha256:" + "a" * 64},
        "operation_id": operation_id,
        "observed_stop": observed_stop,
        "observed_terminal": observed_terminal,
        "expected_terminal": expected,
        "execution": {
            **command(),
            "json_valid": bool(operation_id),
            "terminal_snapshot": observed_stop != "unavailable",
            "reported_success": observed_stop == "completed",
            "infrastructure_error": None,
        },
        "workspace": {
            "base_tree_digest": "sha256:" + "1" * 64,
            "diff": "diff" if expected == "verified" else "",
            "diff_digest": "sha256:" + "b" * 64,
            "changed_files": ["src/lib.rs"] if expected == "verified" else [],
            "forbidden_paths_unchanged": True,
            "required_scope_satisfied": True,
            "dirty_patch_digest": None,
            "dirty_patch_preserved": True,
        },
        "acceptance": [command()],
        "evidence": [
            {
                "operation_id": operation_id,
                "kind": "terminal_snapshot",
                "observed_stop": observed_stop,
            },
            {
                "operation_id": operation_id,
                "kind": "acceptance_command",
                "exit_code": 0,
            },
        ],
        "resources": {
            "passed": True,
            "checks": [{"name": "no_descendant_processes", "passed": True}],
        },
        "metrics": {
            "iterations": metric,
            "tool_calls": metric,
            "tool_errors": 0 if metric is not None else None,
            "inference_rounds": None,
            "provider_retries": 0 if metric is not None else None,
            "active_context_tokens": None,
            "elapsed_ms": metric,
        },
        "failure": {"class": "none", "reasons": []},
        "verification": {"passed": True},
    }
    if failure_mutation is not None:
        failure_mutation(value)
    failure_class, reasons = receipt.classify_failure(value)
    value["failure"] = {"class": failure_class, "reasons": reasons}
    value["verification"] = {"passed": failure_class == "none"}
    return receipt.seal(value)


class SuiteTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, value: dict) -> pathlib.Path:
        path = self.root / f"{value['task_id']}.json"
        path.write_text(json.dumps(value))
        return path

    def test_installed_suite_holds_shared_runtime_generation_lease(self):
        lock_path = self.root / "runtime-mutation.lock"
        environment = {
            "ALETHEON_BIN": "/usr/bin/aletheon",
            "ALETHEON_RUNTIME_LOCK_FILE": str(lock_path),
        }
        with suite.installed_runtime_lease(environment):
            with lock_path.open("a+b") as contender:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(
                        contender.fileno(),
                        fcntl.LOCK_EX | fcntl.LOCK_NB,
                    )

    def test_non_installed_suite_does_not_create_runtime_lease(self):
        lock_path = self.root / "runtime-mutation.lock"
        with suite.installed_runtime_lease(
            {
                "ALETHEON_BIN": str(self.root / "debug-aletheon"),
                "ALETHEON_RUNTIME_LOCK_FILE": str(lock_path),
            }
        ):
            pass
        self.assertFalse(lock_path.exists())

    def test_report_is_deterministic_and_preserves_receipt_integrity(self):
        alpha = self.write(make_receipt("alpha", metric=10))
        beta = self.write(make_receipt("beta", category="lint", metric=30))
        first = suite.aggregate_receipts([beta, alpha])
        second = suite.aggregate_receipts([alpha, beta])
        self.assertEqual(first, second)
        self.assertTrue(receipt.verify_integrity(first))
        self.assertEqual([item["task_id"] for item in first["tasks"]], ["alpha", "beta"])
        self.assertEqual(
            [item["receipt_integrity"] for item in first["tasks"]],
            [
                make_receipt("alpha", metric=10)["integrity_sha256"],
                make_receipt("beta", category="lint", metric=30)["integrity_sha256"],
            ],
        )

    def test_counts_false_success_and_expected_non_success_separately(self):
        passed = self.write(make_receipt("passed"))
        blocked = self.write(
            make_receipt(
                "blocked",
                category="approval",
                expected="blocked",
                observed_stop="blocked",
                observed_terminal="blocked",
            )
        )

        def acceptance_failure(value):
            value["acceptance"][0]["exit_code"] = 1
            value["evidence"][1]["exit_code"] = 1

        false_success = self.write(
            make_receipt("false_success", failure_mutation=acceptance_failure)
        )
        report = suite.aggregate_receipts([false_success, blocked, passed])
        summary = report["summary"]
        self.assertEqual(summary["total"], 3)
        self.assertEqual(summary["benchmark_outcomes_passed"], 2)
        self.assertEqual(summary["completed_engineering_tasks"], 1)
        self.assertEqual(summary["expected_non_success_outcomes"], 1)
        self.assertEqual(summary["false_success_count"], 1)
        self.assertEqual(summary["by_failure_class"]["verification_failure"], 1)
        self.assertEqual(summary["by_category"]["approval"]["passed"], 1)
        self.assertEqual(suite.exit_code(report), 1)

    def test_metrics_preserve_null_availability_and_nearest_rank_percentiles(self):
        paths = [
            self.write(make_receipt("one", metric=10)),
            self.write(make_receipt("two", metric=None)),
            self.write(make_receipt("three", metric=30)),
        ]
        metrics = suite.aggregate_receipts(paths)["metrics"]
        self.assertEqual(
            metrics["tool_calls"],
            {"available": 2, "unavailable": 1, "average": 20.0, "p50": 10, "p95": 30},
        )
        self.assertEqual(metrics["inference_rounds"]["available"], 0)
        self.assertIsNone(metrics["inference_rounds"]["average"])
        self.assertIsNone(metrics["active_context_tokens"]["p95"])

    def test_infrastructure_and_invalid_receipts_use_exit_two(self):
        infrastructure = make_receipt(
            "infra",
            observed_stop="unavailable",
            observed_terminal="unavailable",
            operation_id="",
        )
        infrastructure_path = self.write(infrastructure)
        report = suite.aggregate_receipts([infrastructure_path])
        self.assertEqual(report["summary"]["by_failure_class"]["infrastructure_failure"], 1)
        self.assertEqual(suite.exit_code(report), 2)

        tampered = copy.deepcopy(make_receipt("tampered"))
        tampered["metrics"]["tool_calls"] = 999
        tampered_path = self.write(tampered)
        invalid = suite.aggregate_receipts([tampered_path])
        self.assertEqual(invalid["tasks"][0]["failure_class"], "infrastructure_failure")
        self.assertEqual(invalid["tasks"][0]["reasons"], ["receipt_invalid"])
        self.assertEqual(suite.exit_code(invalid), 2)

        report_path = self.root / "empty-suite.json"
        empty_catalog = self.root / "empty-catalog"
        empty_catalog.mkdir()
        empty, code = suite.execute_suite(
            empty_catalog,
            self.root / "empty-receipts",
            report_path,
            root=self.root,
        )
        self.assertEqual(code, 2)
        self.assertTrue(report_path.is_file())
        self.assertEqual(empty["tasks"][0]["reasons"], ["catalog_invalid"])


if __name__ == "__main__":
    unittest.main()
