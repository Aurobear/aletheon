#!/usr/bin/env python3
"""Deterministic acceptance scoreboard tests: build_scoreboard, gate_verdict, project_task."""

from __future__ import annotations

import copy
import hashlib
from datetime import datetime, timezone
import sys
import unittest
from pathlib import Path

REFERENCE_TIME = datetime(2025, 1, 1, tzinfo=timezone.utc)

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"
sys.path.insert(0, str(HARNESS))

from acceptance_scoreboard import (
    _ENTRY_KEYS,
    _REPORT_KEYS,
    build_scoreboard,
    gate_verdict,
    project_task,
)
from acceptance_contract import ALLOWED_STATUSES, ENVIRONMENT_LEVELS, ContractError
from receipt import digest, seal, verify_integrity


def _sha256_hex(s: str) -> str:
    return hashlib.sha256(s.encode()).hexdigest()


def _make_provenance() -> dict:
    """Build a valid provenance matching acceptance_contract.py exactly."""
    fixture_digest = _sha256_hex("fixture")
    installed_sha = _sha256_hex("installed")
    return {
        "repo": {"sha": _sha256_hex("repo"), "dirty": False},
        "build": {"profile": "release", "features": []},
        "environment_level": "installed",
        "installed_artifact": {"path": "/usr/bin/aletheon", "sha256": installed_sha},
        # client: {version, protocol_version} only (no sha256)
        "client": {
            "version": "1.0.0",
            "protocol_version": "1",
        },
        "daemons": {
            # Both daemon paths must be /usr/bin/aletheon for installed
            "machine": {
                "path": "/usr/bin/aletheon",
                "sha256": installed_sha,
                "version": "1.0.0",
                "protocol_version": "1",
            },
            "user": {
                "path": "/usr/bin/aletheon",
                "sha256": installed_sha,
                "version": "1.0.0",
                "protocol_version": "1",
            },
        },
        # provider: {provider_id, model_id, endpoint_id} only
        "provider": {
            "provider_id": "test-provider",
            "model_id": "test-model",
            "endpoint_id": "test-endpoint",
        },
        "fixture_digest": fixture_digest,
        "generation_id": "gen.test.1",
    }


def _make_passing_task(task_id: str, generation_id: str) -> dict:
    """Return a single passing task entry."""
    return {
        "task_id": task_id,
        "category": "behavioral_bugfix",
        "receipt_valid": True,
        "execution_present": True,
        "outcome_passed": True,
        "failure_class": "none",
        "reasons": [],
        "scope_violation_count": 0,
        "resource_leak_count": 0,
        "terminal_settlement_count": 1,
        "retry_count": 0,
        "started_at": "2025-01-01T00:00:00Z",
        "ended_at": "2025-01-01T00:01:00Z",
        "exit_code": 0,
        "expected_terminal": "verified",
        "observed_terminal": "verified",
        "evidence_paths": [f"logs/{task_id}.log"],
        "generation_id": generation_id,
        "p0": False,
        "waiver": None,
    }


def _make_failed_task(task_id: str, generation_id: str, p0: bool = False) -> dict:
    """Return a single non-P0 failed task entry."""
    return {
        "task_id": task_id,
        "category": "behavioral_bugfix",
        "receipt_valid": False,
        "execution_present": True,
        "outcome_passed": False,
        "failure_class": "execution_failure",
        "reasons": ["timeout"],
        "scope_violation_count": 0,
        "resource_leak_count": 0,
        "terminal_settlement_count": 1,
        "retry_count": 1,
        "started_at": "2025-01-01T00:00:00Z",
        "ended_at": "2025-01-01T00:01:00Z",
        "exit_code": 1,
        "expected_terminal": "verified",
        "observed_terminal": "verified",
        "evidence_paths": [f"logs/{task_id}.log"],
        "generation_id": generation_id,
        "p0": p0,
        "waiver": None,
    }


def _make_multi_report(provenance: dict, tasks: list[dict]) -> dict:
    """Build a sealed report from a provenance and task list."""
    installed_sha = provenance["installed_artifact"]["sha256"]
    fixture_digest = provenance["fixture_digest"]
    report = {
        "schema_version": 1,
        "receipt_schema_version": 2,
        "catalog": {
            "task_schema_version": 1,
            "task_ids": [t["task_id"] for t in tasks],
            "digest": "sha256:" + fixture_digest,
        },
        "binaries": [
            {"path": "/usr/bin/aletheon", "sha256": "sha256:" + installed_sha}
        ],
        "summary": {
            "total": len(tasks),
            "benchmark_outcomes_passed": len(tasks),
            "scope_violation_count": 0,
            "leaked_resource_count": 0,
        },
        "metrics": {"some_metric": 1},
        "tasks": tasks,
    }
    sealed = seal(report)
    return sealed


class AcceptanceScoreboardTest(unittest.TestCase):
    """Tests for build_scoreboard, gate_verdict, and project_task."""

    # -- basic build_scoreboard tests ------------------------------------

    def test_build_scoreboard_rejects_invalid_report_keys(self):
        prov = _make_provenance()
        report = {"bad": 1}
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    def test_build_scoreboard_with_valid_inputs(self):
        prov = _make_provenance()
        gen_id = prov["generation_id"]
        tasks = [_make_passing_task("task-1", gen_id)]
        report = _make_multi_report(prov, tasks)
        sb = build_scoreboard("run-1", report, prov)
        self.assertEqual(sb["run_id"], "run-1")
        self.assertEqual(sb["schema_version"], 1)
        self.assertEqual(len(sb["tasks"]), 1)
        self.assertEqual(sb["tasks"][0]["status"], "passed")

    def test_build_scoreboard_validates_provenance(self):
        prov = _make_provenance()
        # Corrupt: add extra key to client
        prov["client"]["sha256"] = _sha256_hex("bad")
        tasks = [_make_passing_task("task-1", prov["generation_id"])]
        report = _make_multi_report(prov, tasks)
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    def test_build_scoreboard_requires_report_integrity(self):
        prov = _make_provenance()
        tasks = [_make_passing_task("task-1", prov["generation_id"])]
        report = _make_multi_report(prov, tasks)
        # Tamper summary
        report["summary"]["total"] = 999
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    def test_generation_id_must_match_tasks(self):
        prov = _make_provenance()
        tasks = [_make_passing_task("task-1", "wrong-gen")]
        report = _make_multi_report(prov, tasks)
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    def test_missing_tasks_field(self):
        prov = _make_provenance()
        report = _make_multi_report(prov, [_make_passing_task("task-1", prov["generation_id"])])
        del report["tasks"]
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    def test_tasks_not_list(self):
        prov = _make_provenance()
        report = _make_multi_report(prov, [_make_passing_task("task-1", prov["generation_id"])])
        report["tasks"] = "not-a-list"
        with self.assertRaises(ContractError):
            build_scoreboard("run", report, prov)

    # -- gate_verdict tests (20-task gate semantics) --------------------
    # gate_verdict operates on PROJECTED tasks (with 'status' key).

    def _make_20_projected(self, gen_id, num_failed=0, p0_fail=False):
        """Build a list of 20 projected tasks: (20-num_failed) passed, num_failed failed."""
        tasks = []
        for i in range(20 - num_failed):
            entry = _make_passing_task(f"task-{i+1:02d}", gen_id)
            tasks.append(project_task(entry, gen_id))
        for j in range(num_failed):
            is_p0 = p0_fail and (j == 0)
            entry = _make_failed_task(f"task-fail-{j+1:02d}", gen_id, p0=is_p0)
            tasks.append(project_task(entry, gen_id))
        return tasks

    def test_gate_20_all_passed(self):
        """20 tasks all passed => gate passed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["reasons"], [])

    def test_gate_19_passed_1_nonp0_failed(self):
        """19 passed + 1 non-P0 failed => gate passed."""
        tasks = self._make_20_projected("gen1", num_failed=1, p0_fail=False)
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["reasons"], [])

    def test_gate_18_passed_fails(self):
        """18 passed is insufficient => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=2, p0_fail=False)
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("insufficient_passed", result["reasons"])

    def test_gate_task_count_mismatch(self):
        """Not exactly 20 tasks => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks = tasks[:19]  # 19 tasks
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("task_count", result["reasons"])

    def test_gate_p0_failed(self):
        """P0 failed => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=1, p0_fail=True)
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("p0_failure", result["reasons"])

    def test_gate_not_run_nonterminal(self):
        """A not_run task makes gate fail (nonterminal)."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        # Replace one task with a not_run projected entry
        tasks[0] = {
            "task_id": "task-notrun",
            "category": "test",
            "status": "not_run",
            "receipt_valid": False,
            "execution_present": False,
            "outcome_passed": False,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": None,
            "ended_at": None,
            "exit_code": None,
            "expected_terminal": "verified",
            "observed_terminal": None,
            "evidence_paths": [],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("nonterminal_tasks", result["reasons"])

    def test_gate_infra_blocked_nonterminal(self):
        """An infra_blocked task makes gate fail (nonterminal)."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0] = {
            "task_id": "task-infra",
            "category": "test",
            "status": "infra_blocked",
            "receipt_valid": False,
            "execution_present": True,
            "outcome_passed": False,
            "failure_class": "infrastructure_failure",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 22,
            "expected_terminal": "verified",
            "observed_terminal": "provider_unavailable",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("nonterminal_tasks", result["reasons"])

    def test_gate_settlement_mismatch(self):
        """Settlement count != 20 => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0]["terminal_settlement_count"] = 0
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("settlement_mismatch", result["reasons"])

    def test_gate_scope_violation(self):
        """Scope > 0 => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0]["scope_violation_count"] = 1
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("scope_violation", result["reasons"])

    def test_gate_resource_leak(self):
        """Resource leak > 0 => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0]["resource_leak_count"] = 1
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("resource_leak", result["reasons"])

    def test_gate_generation_mismatch(self):
        """Mixed generation IDs => gate failed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0]["generation_id"] = "gen2"
        result = gate_verdict(tasks)
        self.assertEqual(result["status"], "failed")
        self.assertIn("generation_mismatch", result["reasons"])

    def test_gate_empty_generation(self):
        """Empty generation ID is structurally malformed."""
        tasks = self._make_20_projected("gen1", num_failed=0)
        tasks[0]["generation_id"] = ""
        with self.assertRaises(ContractError):
            gate_verdict(tasks)

    # -- project_task: not_run semantics ----------------------------------

    def test_project_task_not_executed_is_not_run(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": False,
            "outcome_passed": False,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": None,
            "ended_at": None,
            "exit_code": None,
            "expected_terminal": "verified",
            "observed_terminal": None,
            "evidence_paths": [],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "not_run")

    def test_project_task_not_executed_cannot_have_outcome(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": False,
            "outcome_passed": True,  # invalid
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": None,
            "ended_at": None,
            "exit_code": None,
            "expected_terminal": "verified",
            "observed_terminal": None,
            "evidence_paths": [],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        with self.assertRaises(ContractError):
            project_task(entry, "gen1")

    # -- project_task: infrastructure_failure => infra_blocked -----------

    def test_project_task_infrastructure_failure_is_infra_blocked(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": False,
            "failure_class": "infrastructure_failure",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 22,
            "expected_terminal": "verified",
            "observed_terminal": "provider_unavailable",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "infra_blocked")

    # -- project_task: passed strict conditions ---------------------------

    def test_project_task_passed_all_conditions(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "passed")

    def test_project_task_passed_requires_exit_zero(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 1,  # non-zero => failed
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    def test_expected_blocked_and_budget_exit_twenty_pass(self):
        for expected in ("blocked", "budget_exhausted"):
            with self.subTest(expected=expected):
                entry = _make_passing_task("t1", "gen1")
                entry.update(
                    expected_terminal=expected,
                    observed_terminal=expected,
                    exit_code=20,
                )
                result = project_task(entry, "gen1")
                self.assertEqual(result["status"], "passed")

    def test_expected_cancelled_exit_twenty_one_passes(self):
        entry = _make_passing_task("t1", "gen1")
        entry.update(
            expected_terminal="cancelled",
            observed_terminal="cancelled",
            exit_code=21,
        )
        self.assertEqual(project_task(entry, "gen1")["status"], "passed")

    def test_expected_terminal_rejects_arbitrary_nonzero_exit(self):
        entry = _make_passing_task("t1", "gen1")
        entry.update(
            expected_terminal="blocked",
            observed_terminal="blocked",
            exit_code=7,
        )
        self.assertEqual(project_task(entry, "gen1")["status"], "failed")

    def test_terminal_mismatch_cannot_pass_with_canonical_exit(self):
        entry = _make_passing_task("t1", "gen1")
        entry.update(
            expected_terminal="blocked",
            observed_terminal="budget_exhausted",
            exit_code=20,
        )
        self.assertEqual(project_task(entry, "gen1")["status"], "failed")

    def test_project_task_passed_requires_settlement_one(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 2,  # not 1 => failed
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    def test_project_task_passed_requires_zero_scope(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 1,  # >0 => failed
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    def test_project_task_passed_requires_zero_leak(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 1,  # >0 => failed
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    # -- project_task: executed invalid receipt => failed -----------------

    def test_project_task_invalid_receipt_failed(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": True,
            "outcome_passed": False,
            "failure_class": "execution_failure",
            "reasons": ["timeout"],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 1,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 1,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": None,
        }
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    def test_project_task_invalid_executed_receipt_allows_unknown_exit(self):
        entry = _make_failed_task("t1", "gen1")
        entry.update(
            failure_class="infrastructure_failure",
            reasons=["receipt_invalid"],
            retry_count=0,
            terminal_settlement_count=0,
            exit_code=None,
            observed_terminal=None,
        )
        result = project_task(entry, "gen1")
        self.assertEqual(result["status"], "failed")

    # -- waiver tests ----------------------------------------------------

    def test_project_task_not_run_with_waiver(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": False,
            "outcome_passed": False,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": None,
            "ended_at": None,
            "exit_code": None,
            "expected_terminal": "verified",
            "observed_terminal": None,
            "evidence_paths": [],
            "generation_id": "gen1",
            "p0": False,
            "waiver": {
                "approver": "auditor",
                "reason": "not needed",
                "expires_at": "2026-01-01T00:00:00Z",
            },
        }
        result = project_task(
            entry,
            "gen1",
            reference_time=REFERENCE_TIME,
        )
        self.assertEqual(result["status"], "waived")

    def test_project_task_p0_waiver_rejected(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": True,
            "outcome_passed": False,
            "failure_class": "execution_failure",
            "reasons": ["timeout"],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 1,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 1,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": True,
            "waiver": {
                "approver": "auditor",
                "reason": "waived anyway",
                "expires_at": "2026-01-01T00:00:00Z",
            },
        }
        with self.assertRaises(ContractError):
            project_task(entry, "gen1")

    def test_project_task_infra_blocked_with_waiver(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": False,
            "execution_present": True,
            "outcome_passed": False,
            "failure_class": "infrastructure_failure",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 0,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 1,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            "waiver": {
                "approver": "auditor",
                "reason": "infra issue",
                "expires_at": "2026-01-01T00:00:00Z",
            },
        }
        result = project_task(
            entry,
            "gen1",
            reference_time=REFERENCE_TIME,
        )
        self.assertEqual(result["status"], "waived")

    # -- build_scoreboard error does not leak secrets --------------------

    def test_build_scoreboard_error_does_not_leak_secrets(self):
        prov = _make_provenance()
        prov["provider"]["endpoint_id"] = "ep-secret-token-abc"
        tasks = [_make_passing_task("task-1", prov["generation_id"])]
        report = _make_multi_report(prov, tasks)
        try:
            build_scoreboard("run", report, prov)
        except ContractError as e:
            msg = str(e)
            self.assertNotIn("token", msg.lower())
            self.assertNotIn("secret", msg.lower())
            self.assertNotIn("password", msg.lower())

    # -- entry key validation --------------------------------------------

    def test_entry_missing_key_rejected(self):
        entry = {
            "task_id": "t1",
            "category": "test",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": True,
            "failure_class": "none",
            "reasons": [],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": "2025-01-01T00:00:00Z",
            "ended_at": "2025-01-01T00:01:00Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": ["logs/ev"],
            "generation_id": "gen1",
            "p0": False,
            # missing waiver key
        }
        with self.assertRaises(ContractError):
            project_task(entry, "gen1")


if __name__ == "__main__":
    unittest.main()
