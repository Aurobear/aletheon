#!/usr/bin/env python3
"""Deterministic aggregation tests for coding benchmark suite receipts."""
from __future__ import annotations

import copy
import dataclasses
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
acceptance_scoreboard = load_module(
    "suite_acceptance_scoreboard", HARNESS / "acceptance_scoreboard.py"
)


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

    def test_missing_required_change_is_not_reported_as_scope_violation(self):
        def missing_required_change(value):
            value["workspace"]["required_scope_satisfied"] = False
            value["execution"]["reported_success"] = False

        path = self.write(
            make_receipt("missing-change", failure_mutation=missing_required_change)
        )
        report = suite.aggregate_receipts([path])
        task = report["tasks"][0]
        self.assertEqual(task["failure_class"], "policy_scope_failure")
        self.assertEqual(task["reasons"], ["required_scope_not_satisfied"])
        self.assertFalse(task["scope_violation"])
        self.assertFalse(suite._is_p0_failure(task))
        self.assertEqual(report["summary"]["scope_violation_count"], 0)

    def test_forbidden_write_is_reported_as_scope_violation(self):
        def forbidden_write(value):
            value["workspace"]["forbidden_paths_unchanged"] = False

        path = self.write(
            make_receipt("forbidden-write", failure_mutation=forbidden_write)
        )
        report = suite.aggregate_receipts([path])
        task = report["tasks"][0]
        self.assertEqual(task["reasons"], ["forbidden_path_changed"])
        self.assertTrue(task["scope_violation"])
        self.assertTrue(suite._is_p0_failure(task))
        self.assertEqual(report["summary"]["scope_violation_count"], 1)

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

    def test_invalid_executed_receipt_projects_as_structural_failure(self):
        catalog = self.root / "tests/coding/tasks"
        fixture = self.root / "tests/coding/fixtures/sample"
        hidden = self.root / "tests/coding/acceptance/sample"
        catalog.mkdir(parents=True)
        fixture.mkdir(parents=True)
        hidden.mkdir(parents=True)
        (fixture / "input.txt").write_text("fixture\n", encoding="utf-8")
        (hidden / "rubric.toml").write_text("schema_version = 1\n", encoding="utf-8")
        (catalog / "sample.toml").write_text(
            """schema_version = 1
id = "sample"
category = "lint"
fixture = "sample"
prompt = "make the smallest safe change"
timeout_secs = 30
acceptance_commands = [["true"]]
forbidden_paths = []
required_changed_paths = []
expected_terminal = "verified"
resource_checks = []

[setup]
""",
            encoding="utf-8",
        )
        runtime = {}

        def write_invalid(_task_path, output, **_kwargs):
            output.write_text("{}\n", encoding="utf-8")
            return {}

        with mock.patch.object(suite, "run_task", side_effect=write_invalid):
            report, code = suite.execute_suite(
                catalog,
                self.root / "receipts",
                self.root / "report.json",
                root=self.root,
                environ={"ALETHEON_BIN": str(self.root / "not-installed")},
                acceptance_generation_id="gen1",
                acceptance_runtime_out=runtime,
            )

        self.assertEqual(code, 2)
        self.assertEqual(report["tasks"][0]["expected_terminal"], "verified")
        self.assertEqual(report["tasks"][0]["category"], "lint")
        acceptance = suite._acceptance_report(report, runtime, "gen1")
        projected_input = acceptance["tasks"][0]
        self.assertTrue(projected_input["execution_present"])
        self.assertFalse(projected_input["receipt_valid"])
        self.assertIsNone(projected_input["exit_code"])
        self.assertEqual(projected_input["evidence_paths"], ["logs/sample.json"])
        projected = acceptance_scoreboard.project_task(projected_input, "gen1")
        self.assertEqual(projected["status"], "failed")

        installed_sha = "a" * 64
        report["binaries"] = [
            {"path": "/usr/bin/aletheon", "sha256": "sha256:" + installed_sha}
        ]
        acceptance = suite._acceptance_report(report, runtime, "gen1")
        provenance = {
            "repo": {"sha": "b" * 40, "dirty": False},
            "build": {"profile": "release", "features": []},
            "environment_level": "installed",
            "installed_artifact": {
                "path": "/usr/bin/aletheon",
                "sha256": installed_sha,
            },
            "client": {"version": "1.0.0", "protocol_version": "1"},
            "daemons": {
                name: {
                    "path": "/usr/bin/aletheon",
                    "sha256": installed_sha,
                    "version": "1.0.0",
                    "protocol_version": "1",
                }
                for name in ("machine", "user")
            },
            "provider": {
                "provider_id": "test-provider",
                "model_id": "test-model",
                "endpoint_id": "test-endpoint",
            },
            "fixture_digest": report["catalog"]["digest"][len("sha256:"):],
            "generation_id": "gen1",
        }
        artifacts = self.root / "artifacts"
        artifacts.mkdir()
        run_dir = suite.write_run(
            "invalid-receipt",
            acceptance,
            provenance,
            artifacts,
            evidence_root=self.root / "receipts",
        )
        scoreboard = json.loads((run_dir / "scoreboard.json").read_text())
        self.assertEqual(scoreboard["tasks"][0]["status"], "failed")
        self.assertEqual(scoreboard["gate"]["status"], "failed")


class AcceptanceInputDigestTest(unittest.TestCase):
    """The published input digest binds every executable acceptance input."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.task_source = self.root / "tests/coding/tasks/sample.toml"
        self.fixture = self.root / "tests/coding/fixtures/sample"
        self.acceptance = self.root / "tests/coding/acceptance/sample"
        self.task_source.parent.mkdir(parents=True)
        self.fixture.mkdir(parents=True)
        self.acceptance.mkdir(parents=True)
        self.task_source.write_text('prompt = "before"\n', encoding="utf-8")
        (self.fixture / "source.txt").write_text("fixture\n", encoding="utf-8")
        (self.acceptance / "rubric.toml").write_text("score = 1\n", encoding="utf-8")
        self.task = suite.BenchmarkTask(
            schema_version=1,
            id="sample",
            category="behavioral_bugfix",
            fixture="sample",
            prompt="before",
            timeout_secs=30,
            acceptance_commands=(("true",),),
            forbidden_paths=(),
            required_changed_paths=(),
            expected_terminal="verified",
            setup={},
            resource_checks=(),
            source=self.task_source,
        )

    def tearDown(self):
        self.temp.cleanup()

    def digest(self) -> str:
        return suite.acceptance_input_digest([self.task], self.root)

    def test_digest_is_deterministic_when_inputs_are_unchanged(self):
        self.assertEqual(self.digest(), self.digest())

    def test_task_prompt_change_changes_digest(self):
        before = self.digest()
        self.task_source.write_text('prompt = "after"\n', encoding="utf-8")
        self.assertNotEqual(before, self.digest())

    def test_fixture_file_change_changes_digest(self):
        before = self.digest()
        (self.fixture / "source.txt").write_text("changed\n", encoding="utf-8")
        self.assertNotEqual(before, self.digest())

    def test_acceptance_rubric_change_changes_digest(self):
        before = self.digest()
        (self.acceptance / "rubric.toml").write_text("score = 2\n", encoding="utf-8")
        self.assertNotEqual(before, self.digest())

    def test_symlinked_input_is_rejected(self):
        (self.fixture / "linked.txt").symlink_to(self.fixture / "source.txt")
        with self.assertRaises(suite.ContractError):
            self.digest()

    def test_repository_escape_is_rejected(self):
        outside = self.root.parent / f"{self.root.name}-outside.toml"
        outside.write_text('prompt = "outside"\n', encoding="utf-8")
        self.addCleanup(outside.unlink, missing_ok=True)
        escaped = dataclasses.replace(self.task, source=outside)
        with self.assertRaises(suite.ContractError):
            suite.acceptance_input_digest([escaped], self.root)

    def test_catalog_directory_symlink_is_rejected_before_resolution(self):
        alias = self.root / "catalog-alias"
        alias.symlink_to(self.task_source.parent, target_is_directory=True)
        with self.assertRaises(suite.ContractError):
            suite._checked_repository_path(self.root, alias, expected="directory")

    def test_oversized_file_is_rejected(self):
        with mock.patch.object(suite, "MAX_ACCEPTANCE_INPUT_FILE_BYTES", 3):
            with self.assertRaises(suite.ContractError):
                self.digest()

    def test_excessive_entry_count_is_rejected(self):
        with mock.patch.object(suite, "MAX_ACCEPTANCE_INPUT_ENTRIES", 1):
            with self.assertRaises(suite.ContractError):
                self.digest()

    def test_total_input_size_is_bounded(self):
        with mock.patch.object(suite, "MAX_ACCEPTANCE_INPUT_TOTAL_BYTES", 1):
            with self.assertRaises(suite.ContractError):
                self.digest()

    def test_symlink_replacement_before_open_is_rejected(self):
        real_open = os.open
        replaced = False

        def replace_then_open(path, flags, *args, **kwargs):
            nonlocal replaced
            if pathlib.Path(path) == self.task_source and not replaced:
                replaced = True
                self.task_source.unlink()
                self.task_source.symlink_to(self.fixture / "source.txt")
            return real_open(path, flags, *args, **kwargs)

        with mock.patch.object(suite.os, "open", side_effect=replace_then_open):
            with self.assertRaises(suite.ContractError):
                self.digest()
        self.assertTrue(replaced)

    def test_multifile_prompt_binds_comma_only_serialization(self):
        prompt = (
            pathlib.Path(__file__).parent / "tasks/rust_multifile.toml"
        ).read_text(encoding="utf-8")
        self.assertIn("使用英文逗号直接连接结果（项目之间不插入空格）", prompt)


class AcceptanceSourceGuardTest(unittest.TestCase):
    def setUp(self):
        self.before = {
            "repo_sha": "a" * 40,
            "repo_dirty": False,
            "repo_status_digest": "sha256:" + "b" * 64,
            "input_digest": "sha256:" + "c" * 64,
        }

    def test_unchanged_snapshot_is_accepted(self):
        suite.assert_acceptance_source_unchanged(self.before, dict(self.before))

    def test_head_dirty_and_input_mutations_are_rejected(self):
        mutations = {
            "repo_sha": "d" * 40,
            "repo_dirty": True,
            "repo_status_digest": "sha256:" + "e" * 64,
            "input_digest": "sha256:" + "f" * 64,
        }
        for field, value in mutations.items():
            with self.subTest(field=field):
                after = dict(self.before)
                after[field] = value
                with self.assertRaises(suite.ContractError):
                    suite.assert_acceptance_source_unchanged(self.before, after)

    def test_provenance_must_match_source_identity_and_digest(self):
        provenance = {
            "repo": {"sha": self.before["repo_sha"], "dirty": False},
            "fixture_digest": str(self.before["input_digest"])[len("sha256:"):],
        }
        suite.assert_acceptance_provenance_matches_source(self.before, provenance)
        for mutation in (
            {"repo": {"sha": "d" * 40, "dirty": False}},
            {"repo": {"sha": self.before["repo_sha"], "dirty": True}},
            {"fixture_digest": "e" * 64},
        ):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(provenance)
                changed.update(mutation)
                with self.assertRaises(suite.ContractError):
                    suite.assert_acceptance_provenance_matches_source(
                        self.before, changed
                    )


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
        self.assertEqual(result["expected_terminal"], "verified")
        self.assertEqual(result["observed_terminal"], "verified")

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

    def test_invalid_executed_receipt_preserves_evidence_path(self):
        """An invalid generated receipt remains evidence of a failed execution."""
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
        self.assertEqual(result["evidence_paths"], ["logs/t3.json"])
        self.assertTrue(result["execution_present"])


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
        self._source_snapshot_patch = mock.patch.object(
            suite, "capture_acceptance_source_state"
        )
        self.mock_source_snapshot = self._source_snapshot_patch.start()
        self.mock_source_snapshot.return_value = {
            "repo_sha": "a" * 40,
            "repo_dirty": False,
            "repo_status_digest": "sha256:" + "b" * 64,
            "input_digest": "sha256:" + "c" * 64,
        }

    def tearDown(self):
        self._source_snapshot_patch.stop()
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

    def _matching_provenance(self, report):
        return {
            "repo": {
                "sha": self.mock_source_snapshot.return_value["repo_sha"],
                "dirty": self.mock_source_snapshot.return_value["repo_dirty"],
            },
            "generation_id": "gen1",
            "fixture_digest": report["catalog"]["digest"][len("sha256:"):],
        }

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
        self.mock_source_snapshot.return_value["input_digest"] = report["catalog"]["digest"]

        mock_exec.return_value = (report, 0)
        mock_collect.return_value = self._matching_provenance(report)
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
    def test_source_mutation_preserves_report_but_aborts_artifact_emission(
        self, mock_exec, mock_write, mock_collect,
    ):
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        before = dict(self.mock_source_snapshot.return_value)
        before["input_digest"] = report["catalog"]["digest"]
        after = dict(before)
        after["input_digest"] = "sha256:" + "d" * 64
        self.mock_source_snapshot.side_effect = [before, after]

        def execute(*_args, **_kwargs):
            suite._write_report(report, self.report_path)
            return report, 0

        mock_exec.side_effect = execute
        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py",
            "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-mutated",
            "--generation-id", "gen1",
            "--artifacts-root", str(self.root / "artifacts"),
        ]

        with self.assertRaises(SystemExit) as ctx:
            suite.main()
        self.assertEqual(ctx.exception.code, 2)
        self.assertTrue(self.report_path.is_file())
        mock_collect.assert_not_called()
        mock_write.assert_not_called()

    @mock.patch.object(suite, "collect_installed_provenance")
    @mock.patch.object(suite, "write_run")
    @mock.patch.object(suite, "execute_suite")
    def test_provenance_repo_mismatch_aborts_artifact_emission(
        self, mock_exec, mock_write, mock_collect,
    ):
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        self.mock_source_snapshot.return_value["input_digest"] = report["catalog"]["digest"]
        mock_exec.return_value = (report, 0)
        mock_collect.return_value = self._matching_provenance(report)
        mock_collect.return_value["repo"]["sha"] = "d" * 40
        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py", "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-provenance-mismatch",
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
    def test_source_mutation_during_provenance_aborts_artifact_emission(
        self, mock_exec, mock_write, mock_collect,
    ):
        entries = [suite._invalid_entry("t1")]
        entries[0]["category"] = "lint"
        entries[0]["expected_terminal"] = "verified"
        report = suite._build_report(entries, [])
        before = dict(self.mock_source_snapshot.return_value)
        before["input_digest"] = report["catalog"]["digest"]
        after = dict(before)
        after["repo_status_digest"] = "sha256:" + "d" * 64
        self.mock_source_snapshot.side_effect = [before, before, after]
        mock_exec.return_value = (report, 0)
        mock_collect.return_value = {
            "repo": {"sha": before["repo_sha"], "dirty": before["repo_dirty"]},
            "generation_id": "gen1",
            "fixture_digest": report["catalog"]["digest"][len("sha256:"):],
        }
        self._set_env(ALETHEON_BIN="/usr/bin/aletheon")
        sys.argv = [
            "suite.py", "--catalog", str(self.root / "catalog"),
            "--receipts", str(self.root / "receipts"),
            "--report", str(self.report_path),
            "--run-id", "test-run-provenance-race",
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
        self.mock_source_snapshot.return_value["input_digest"] = report["catalog"]["digest"]
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
        self.mock_source_snapshot.return_value["input_digest"] = report["catalog"]["digest"]
        mock_exec.return_value = (report, 0)  # suite code 0 must not leak through
        mock_collect.return_value = self._matching_provenance(report)
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
