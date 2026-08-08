#!/usr/bin/env python3
"""Deterministic aggregation tests for coding benchmark suite receipts."""
from __future__ import annotations

import copy
import fcntl
import importlib.util
import json
import os
import pathlib
import sys
import tempfile
import socket
import unittest
from unittest import mock

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


class AcceptanceTaskTest(unittest.TestCase):
    """Tests for _acceptance_task evidence-path and projection behaviour."""

    def test_evidence_path_is_canonical_logs(self):
        """_acceptance_task evidence_paths uses logs/<name> prefix."""
        entry = {
            "task_id": "t1",
            "category": "lint",
            "receipt_valid": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation": False,
            "resource_leak": False,
            "false_success": False,
        }
        receipt_data = make_receipt("t1", metric=10)

        result = suite._acceptance_task(
            entry,
            receipt_data,
            "logs/t1.json",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
            "gen1",
        )
        self.assertEqual(result["evidence_paths"], ["logs/t1.json"])
        self.assertTrue(result["execution_present"])

    def test_evidence_path_none_gives_empty_list(self):
        """_acceptance_task with receipt_path=None produces empty evidence_paths."""
        entry = {
            "task_id": "t2",
            "category": "lint",
            "receipt_valid": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation": False,
            "resource_leak": False,
            "false_success": False,
        }
        receipt_data = make_receipt("t2", metric=10)

        result = suite._acceptance_task(
            entry, receipt_data, None, None, None, "gen1",
        )
        self.assertEqual(result["evidence_paths"], [])
        # receipt_valid=True + receipt present -> execution_present=True
        # but missing receipt_path flips outcome_passed and adds reason
        self.assertTrue(result["execution_present"])
        self.assertFalse(result["outcome_passed"])
        self.assertIn("receipt_evidence_path_missing", result["reasons"])

    def test_invalid_receipt_empty_evidence(self):
        """_acceptance_task with receipt_valid=False produces empty evidence_paths."""
        entry = {
            "task_id": "t3",
            "category": "lint",
            "receipt_valid": False,
            "outcome_passed": False,
            "failure_class": "infrastructure_failure",
            "reasons": [],
            "scope_violation": False,
            "resource_leak": False,
            "false_success": False,
        }
        result = suite._acceptance_task(
            entry, None, "logs/t3.json",
            "2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z", "gen1",
        )
        self.assertEqual(result["evidence_paths"], [])
        self.assertFalse(result["execution_present"])


class AcceptanceCLITest(unittest.TestCase):
    """Tests for the acceptance CLI mode in main() using mocked dependencies."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        (self.root / "catalog").mkdir()
        (self.root / "receipts").mkdir()
        (self.root / "artifacts").mkdir()
        self.report_path = self.root / "report.json"
        self.runtime_dir = self.root / "runtime"
        self.socket_path = self.runtime_dir / "aletheon" / "aletheon.sock"
        self.socket_path.parent.mkdir(parents=True)
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(self.socket_path))
        self._orig_argv = sys.argv[:]
        self._orig_environ = os.environ.copy()

    def tearDown(self):
        self.listener.close()
        self.temp.cleanup()
        sys.argv = self._orig_argv
        os.environ.clear()
        os.environ.update(self._orig_environ)

    def _set_env(self, **kwargs):
        os.environ.update(
            {
                "XDG_RUNTIME_DIR": str(self.runtime_dir),
                "ALETHEON_ACCEPTANCE_SOCKET": str(self.socket_path),
                **kwargs,
            }
        )

    # ------------------------------------------------------------------
    #  CLI rejection tests
    # ------------------------------------------------------------------

    def test_rejects_partial_acceptance_group_run_id_only(self):
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_missing_artifacts_root(self):
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_missing_generation_id(self):
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_old_provenance_flag(self):
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--provenance", str(self.root / "prov.json"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_bad_aletheon_bin(self):
        self._set_env(ALETHEON_BIN=str(self.root / "bad-binary"))
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_non_official_acceptance_socket(self):
        self._set_env(
            ALETHEON_BIN="/usr/bin/aletheon",
            ALETHEON_ACCEPTANCE_SOCKET=str(self.root / "other.sock"),
        )
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_empty_run_id(self):
        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "   ",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    def test_rejects_empty_generation_id(self):
        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "   ",
            "--artifacts-root", str(self.root / "artifacts"),
        ]
        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)

    # ------------------------------------------------------------------
    #  Happy-path and error-flow tests (mocked)
    # ------------------------------------------------------------------

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_happy_acceptance_flow(
        self, mock_exec, mock_write, mock_collect,
    ):
        """Happy acceptance CLI calls execute_suite, collector, write_run."""
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])

        mock_exec.return_value = (report, 0)
        mock_collect.return_value = {
            "generation_id": "gen1",
            "fixture_digest": "a" * 64,
        }
        mock_write.return_value = self.root / "artifacts" / "test-run-1"

        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 0)

        # execute_suite called with explicit generation id + runtime_out
        mock_exec.assert_called_once()
        exec_kw = mock_exec.call_args[1]
        self.assertEqual(exec_kw["acceptance_generation_id"], "gen1")
        self.assertIsInstance(exec_kw.get("acceptance_runtime_out"), dict)

        # collector called with ROOT, raw 64 hex, generation_id
        catalog_digest = report["catalog"]["digest"]
        raw_hex = catalog_digest[len("sha256:"):]
        mock_collect.assert_called_once_with(
            suite.ROOT, raw_hex, "gen1",
        )

        # write_run called with evidence_root = receipts dir
        mock_write.assert_called_once()
        write_args = mock_write.call_args
        self.assertEqual(write_args[0][0], "test-run-1")  # run_id positional
        self.assertEqual(
            write_args[1]["evidence_root"],
            (self.root / "receipts").resolve(),
        )

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_malformed_catalog_digest_rejected(
        self, mock_exec, mock_write, mock_collect,
    ):
        """Malformed catalog digest fails before collector/write."""
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        report["catalog"]["digest"] = "bad-digest"
        mock_exec.return_value = (report, 0)

        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)
        mock_collect.assert_not_called()
        mock_write.assert_not_called()

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_collector_failure_sanitized(
        self, mock_exec, mock_write, mock_collect,
    ):
        """ProvenanceCollectionError -> sanitized argparse error, no write."""
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        mock_exec.return_value = (report, 0)
        mock_collect.side_effect = suite.ProvenanceCollectionError(
            "secret details"
        )

        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)
        mock_write.assert_not_called()

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_write_failure_fail_closed_even_when_suite_code_zero(
        self, mock_exec, mock_write, mock_collect,
    ):
        """write_run failure -> sanitized argparse exit 2, never reports success."""
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        mock_exec.return_value = (report, 0)  # suite code 0 must not leak through
        mock_collect.return_value = {
            "generation_id": "gen1",
            "fixture_digest": "a" * 64,
        }
        mock_write.side_effect = OSError("disk full")

        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-1",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)
        mock_collect.assert_called_once()
        mock_write.assert_called_once()

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_non_acceptance_mode_no_collector(
        self, mock_exec, mock_write, mock_collect,
    ):
        """Non-acceptance mode never calls collector or write_run."""
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        mock_exec.return_value = (report, 0)

        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 0)
        mock_collect.assert_not_called()
        mock_write.assert_not_called()
        mock_exec.assert_called_once()
        self.assertIsNone(
            mock_exec.call_args[1].get("acceptance_generation_id")
        )


if __name__ == "__main__":
    unittest.main()
