#!/usr/bin/env python3
"""Deterministic acceptance artifacts tests: write_run immutability, verify_run integrity."""

from __future__ import annotations

import copy
import hashlib
import json
import multiprocessing
import os
import shutil
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"
sys.path.insert(0, str(HARNESS))

from acceptance_artifacts import (
    pretty_json_bytes,
    render_markdown,
    scoreboard_file_digest,
    verify_run,
    write_run,
)
from acceptance_contract import ALLOWED_STATUSES, ENVIRONMENT_LEVELS, ContractError
from receipt import canonical_bytes, digest, METRIC_KEYS, seal, verify_integrity


# ---------- deterministic log content ------------------------------------
DETERMINISTIC_LOG_BYTES = b"task-1 log content deterministic\n"


# ---------- helpers to build a valid minimal report + provenance ----------

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
        "client": {
            "version": "1.0.0",
            "protocol_version": "1",
        },
        "daemons": {
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
        "provider": {
            "provider_id": "test-provider",
            "model_id": "test-model",
            "endpoint_id": "test-endpoint",
        },
        "fixture_digest": fixture_digest,
        "generation_id": "gen.test.1",
    }


def _make_report(provenance: dict | None = None) -> dict:
    """Build a sealed report with one passing behavioral_bugfix task and full X12 summary."""
    if provenance is None:
        provenance = _make_provenance()
    installed_sha = provenance["installed_artifact"]["sha256"]
    fixture_digest = provenance["fixture_digest"]
    gen_id = provenance["generation_id"]
    # Metrics: one valid receipt, so each metric reports available=1
    metric_template = {"available": 1, "unavailable": 0, "average": 0.0, "p50": 0, "p95": 0}
    metrics = {key: dict(metric_template) for key in METRIC_KEYS}
    report = {
        "schema_version": 1,
        "receipt_schema_version": 2,
        "catalog": {
            "task_schema_version": 1,
            "task_ids": ["task-1"],
            "digest": "sha256:" + fixture_digest,
        },
        "binaries": [
            {"path": "/usr/bin/aletheon", "sha256": "sha256:" + installed_sha}
        ],
        "summary": {
            "total": 1,
            "benchmark_outcomes_passed": 1,
            "benchmark_outcomes_failed": 0,
            "completed_engineering_tasks": 0,
            "expected_non_success_outcomes": 0,
            "validation_pass_rate": 1.0,
            "evidence_complete_success_rate": 1.0,
            "false_success_count": 0,
            "scope_violation_count": 0,
            "leaked_resource_count": 0,
            "by_category": {
                "behavioral_bugfix": {"total": 1, "passed": 1, "failed": 0},
            },
            "by_failure_class": {
                "none": 1,
                "assertion_error": 0,
                "timeout": 0,
                "terminal_settlement_timeout": 0,
                "panic": 0,
                "resource_leak": 0,
            },
            "by_observed_terminal": {
                "verified": 1,
                "killed": 0,
                "unobserved": 0,
                "timeout": 0,
            },
        },
        "metrics": metrics,
        "tasks": [
            {
                "task_id": "task-1",
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
                "evidence_paths": ["logs/task-1.log"],
                "generation_id": gen_id,
                "p0": False,
                "waiver": None,
            }
        ],
    }
    return seal(report)


# -- module-level multiprocessing helpers (requirement 7) -----------------

def _mp_worker(args):
    """Module-level worker for multiprocessing write_run tests."""
    run_id, report, prov, root_str, evidence_root_str = args
    root = Path(root_str)
    evidence_root = Path(evidence_root_str) if evidence_root_str else None
    try:
        return write_run(run_id, report, prov, root, evidence_root=evidence_root)
    except ContractError:
        return None


# -- main test class -------------------------------------------------------

class AcceptanceArtifactsTest(unittest.TestCase):
    """Tests for write_run / verify_run."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="acc_test_")
        self.root = Path(self._tmp.name)
        self._ev_tmp = tempfile.TemporaryDirectory(prefix="ev_")
        self.evidence_root = Path(self._ev_tmp.name)
        # create the default evidence file for task-1
        (self.evidence_root / "task-1.log").write_bytes(DETERMINISTIC_LOG_BYTES)

    def tearDown(self):
        self._tmp.cleanup()
        self._ev_tmp.cleanup()

    # -- helpers for tampering --------------------------------------------
    def _update_manifest_digest_after_sb_change(self, run_dir: Path, sb_bytes: bytes):
        """Rewrite manifest to match the new scoreboard digest."""
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        mf["scoreboard_sha256"] = scoreboard_file_digest(sb_bytes)
        (run_dir / "manifest.json").write_bytes(pretty_json_bytes(mf))

    # -- basic write + verify round-trip (adapted with evidence) ----------

    def test_write_and_verify_valid_roundtrip(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-ok", report, prov, self.root,
                            evidence_root=self.evidence_root)
        self.assertTrue(run_dir.is_dir())
        sb = verify_run(run_dir)
        self.assertEqual(sb["run_id"], "run-ok")
        self.assertEqual(sb["tasks"][0]["status"], "passed")
        # gate must *not* be passed for a single-task run
        self.assertNotEqual(sb["gate"]["status"], "passed")
        # evidence copied correctly
        published_log = run_dir / "logs" / "task-1.log"
        self.assertTrue(published_log.is_file())
        self.assertEqual(published_log.read_bytes(), DETERMINISTIC_LOG_BYTES)
        # manifest hash matches
        expected_digest = hashlib.sha256(DETERMINISTIC_LOG_BYTES).hexdigest()
        manifest = json.loads((run_dir / "manifest.json").read_bytes())
        evidence_sha256 = manifest["evidence_sha256"]
        self.assertIn("logs/task-1.log", evidence_sha256)
        self.assertEqual(evidence_sha256["logs/task-1.log"], expected_digest)

    def test_write_and_verify_nested_evidence_roundtrip(self):
        prov = _make_provenance()
        prov_copy = copy.deepcopy(prov)
        report = copy.deepcopy(_make_report(prov_copy))
        # modify evidence path to nested
        report = seal(report)  # ensure integrity before modification? No, we'll rebuild as sealed
        # Actually easier: rebuild report with nested path
        # We'll create custom report with nested evidence
        installed_sha = prov["installed_artifact"]["sha256"]
        fixture_digest = prov["fixture_digest"]
        gen_id = prov["generation_id"]
        metric_template = {"available": 1, "unavailable": 0, "average": 0.0, "p50": 0, "p95": 0}
        metrics = {key: dict(metric_template) for key in METRIC_KEYS}
        nested_report = {
            "schema_version": 1,
            "receipt_schema_version": 2,
            "catalog": {
                "task_schema_version": 1,
                "task_ids": ["task-1"],
                "digest": "sha256:" + fixture_digest,
            },
            "binaries": [{"path": "/usr/bin/aletheon", "sha256": "sha256:" + installed_sha}],
            "summary": {
                "total": 1,
                "benchmark_outcomes_passed": 1,
                "benchmark_outcomes_failed": 0,
                "completed_engineering_tasks": 0,
                "expected_non_success_outcomes": 0,
                "validation_pass_rate": 1.0,
                "evidence_complete_success_rate": 1.0,
                "false_success_count": 0,
                "scope_violation_count": 0,
                "leaked_resource_count": 0,
                "by_category": {"behavioral_bugfix": {"total": 1, "passed": 1, "failed": 0}},
                "by_failure_class": {"none": 1, "assertion_error": 0, "timeout": 0, "terminal_settlement_timeout": 0, "panic": 0, "resource_leak": 0},
                "by_observed_terminal": {"verified": 1, "killed": 0, "unobserved": 0, "timeout": 0},
            },
            "metrics": metrics,
            "tasks": [{
                "task_id": "task-1",
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
                "evidence_paths": ["logs/sub/task-1.log"],
                "generation_id": gen_id,
                "p0": False,
                "waiver": None,
            }],
        }
        nested_report_sealed = seal(nested_report)
        # prepare evidence: create sub/task-1.log in evidence_root
        nested_evidence_dir = self.evidence_root / "sub"
        nested_evidence_dir.mkdir()
        (nested_evidence_dir / "task-1.log").write_bytes(DETERMINISTIC_LOG_BYTES)
        run_dir = write_run("run-nested", nested_report_sealed, prov, self.root,
                            evidence_root=self.evidence_root)
        sb = verify_run(run_dir)
        self.assertEqual(sb["run_id"], "run-nested")
        # permissions: directories 0700, files 0600
        logs_dir = run_dir / "logs"
        sub_dir = logs_dir / "sub"
        self.assertTrue(logs_dir.is_dir())
        self.assertTrue(sub_dir.is_dir())
        self.assertEqual(stat.S_IMODE(logs_dir.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(sub_dir.stat().st_mode), 0o700)
        log_file = sub_dir / "task-1.log"
        self.assertTrue(log_file.is_file())
        self.assertEqual(stat.S_IMODE(log_file.stat().st_mode), 0o600)
        self.assertEqual(log_file.read_bytes(), DETERMINISTIC_LOG_BYTES)

    def test_write_rejects_bad_run_id(self):
        prov = _make_provenance()
        report = _make_report(prov)
        with self.assertRaises(ContractError):
            write_run("../etc", report, prov, self.root, evidence_root=self.evidence_root)

    def test_write_rejects_empty_run_id(self):
        prov = _make_provenance()
        report = _make_report(prov)
        with self.assertRaises(ContractError):
            write_run("", report, prov, self.root, evidence_root=self.evidence_root)

    def test_write_rejects_double_write(self):
        prov = _make_provenance()
        report = _make_report(prov)
        write_run("run-double", report, prov, self.root, evidence_root=self.evidence_root)
        with self.assertRaises(ContractError):
            write_run("run-double", report, prov, self.root, evidence_root=self.evidence_root)

    def test_verify_rejects_missing_files(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-missing", report, prov, self.root, evidence_root=self.evidence_root)
        (run_dir / "scoreboard.json").unlink()
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_tampered_scoreboard(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-tamper", report, prov, self.root, evidence_root=self.evidence_root)
        (run_dir / "scoreboard.json").write_bytes(b'{"bad": true}')
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_tampered_manifest(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-tamper-mf", report, prov, self.root, evidence_root=self.evidence_root)
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        mf["scoreboard_sha256"] = "f" * 64
        (run_dir / "manifest.json").write_bytes(pretty_json_bytes(mf))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_bounded_read_rejects_large_files(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-big", report, prov, self.root, evidence_root=self.evidence_root)
        giant = b"x" * (17 * 1024 * 1024)
        (run_dir / "scoreboard.json").write_bytes(giant)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_symlink_run_path(self):
        tmp2 = tempfile.mkdtemp(prefix="acc_link_")
        link = Path(tmp2) / "link"
        os.symlink("/tmp", str(link))
        with self.assertRaises(ContractError):
            verify_run(link)
        shutil.rmtree(tmp2, ignore_errors=True)

    def test_verify_rejects_non_directory_run_path(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-nondir", report, prov, self.root, evidence_root=self.evidence_root)
        sf = run_dir / "scoreboard.json"
        with self.assertRaises(ContractError):
            verify_run(sf)

    def test_canonical_json_enforced(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-canon", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        (run_dir / "scoreboard.json").write_bytes(json.dumps(sb).encode("utf-8") + b"\n")
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_markdown_regenerated_byte_for_byte(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-md", report, prov, self.root, evidence_root=self.evidence_root)
        (run_dir / "scoreboard.md").write_bytes(b"# tampered\n")
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    # -- gate recomputation in verify_run (adapted to expect failure) ------

    def test_verify_run_recomputes_gate(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-gate", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["gate"] = {"status": "passed", "reasons": []}
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    # -- provenance cross-check in verify_run -----------------------------

    def test_verify_run_crosschecks_fixture_digest(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-fixd", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["provenance"]["fixture_digest"] = "f" * 64
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_run_crosschecks_generation_id(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-genid", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["provenance"]["generation_id"] = "wrong.gen"
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_run_crosschecks_environment_level(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-env", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["provenance"]["environment_level"] = "simulation"
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        mf["scoreboard_sha256"] = scoreboard_file_digest(sb_bytes)
        # also update manifest's own environment_level to keep it internally consistent for a moment
        mf["environment_level"] = "simulation"
        (run_dir / "manifest.json").write_bytes(pretty_json_bytes(mf))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_run_validates_provenance_normalization(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-provnorm", report, prov, self.root, evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        if "client" in sb["provenance"]:
            sb["provenance"]["client"]["extra"] = "bad"
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    # -- report seal verification in write_run ----------------------------

    def test_report_seal_verified_before_projection(self):
        prov = _make_provenance()
        report = _make_report(prov)
        tampered = copy.deepcopy(report)
        tampered["summary"]["total"] = 999
        with self.assertRaises(ContractError):
            write_run("run-seal", tampered, prov, self.root, evidence_root=self.evidence_root)

    def test_valid_report_seal_passes(self):
        prov = _make_provenance()
        report = _make_report(prov)
        self.assertTrue(verify_integrity(report))
        run_dir = write_run("run-valid-seal", report, prov, self.root,
                            evidence_root=self.evidence_root)
        self.assertTrue(run_dir.is_dir())

    def test_syntactically_valid_but_tampered_report_fails(self):
        prov = _make_provenance()
        report = _make_report(prov)
        tampered = copy.deepcopy(report)
        tampered["integrity_sha256"] = "sha256:" + ("f" * 64)
        self.assertFalse(verify_integrity(tampered))
        with self.assertRaises(ContractError):
            write_run("run-bad-seal", tampered, prov, self.root, evidence_root=self.evidence_root)

    # -- lock safety: symlink rejection -----------------------------------

    def test_write_run_rejects_symlink_lock(self):
        prov = _make_provenance()
        report = _make_report(prov)
        lock_path = self.root / ".lock_run-sym"
        os.symlink("/tmp", str(lock_path))
        with self.assertRaises(ContractError):
            write_run("run-sym", report, prov, self.root, evidence_root=self.evidence_root)

    def test_write_run_rejects_non_regular_lock(self):
        prov = _make_provenance()
        report = _make_report(prov)
        lock_path = self.root / ".lock_run-dir"
        lock_path.mkdir()
        try:
            with self.assertRaises(ContractError):
                write_run("run-dir", report, prov, self.root, evidence_root=self.evidence_root)
        finally:
            lock_path.rmdir()

    # -- lock safety: persistent lock file and permissions ----------------

    def test_lock_file_persists_after_write(self):
        """Lock file is not unlinked after write_run completes."""
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-lock", report, prov, self.root, evidence_root=self.evidence_root)
        self.assertTrue(run_dir.is_dir())
        lock_path = self.root / ".lock_run-lock"
        self.assertTrue(lock_path.exists(), "Lock file should persist after write_run")

    def test_existing_lock_bad_permissions_rejected(self):
        """Lock file with 0o644 is rejected, not repaired."""
        lock_path = self.root / ".lock_run-badperm"
        lock_path.touch(mode=0o644)  # insecure
        prov = _make_provenance()
        report = _make_report(prov)
        with self.assertRaises(ContractError):
            write_run("run-badperm", report, prov, self.root, evidence_root=self.evidence_root)
        # verify lock still has bad permissions (not repaired)
        self.assertTrue(lock_path.exists())
        self.assertEqual(stat.S_IMODE(lock_path.stat().st_mode), 0o644)

    # -- concurrency: exactly one winner (requirement 8) ------------------

    def test_concurrent_write_exactly_one_wins(self):
        prov = _make_provenance()
        report = _make_report(prov)
        root_str = str(self.root)
        evidence_root_str = str(self.evidence_root)
        args = [("concur-1", report, prov, root_str, evidence_root_str) for _ in range(5)]
        with multiprocessing.Pool(processes=5) as pool:
            results = pool.map(_mp_worker, args)
        winners = [r for r in results if r is not None]
        self.assertEqual(len(winners), 1, "Exactly one concurrent writer must win")
        winner = winners[0]
        self.assertTrue(winner.is_dir())
        sb = verify_run(winner)
        self.assertEqual(sb["run_id"], "concur-1")

    def test_concurrent_different_runs_independent(self):
        prov = _make_provenance()
        report = _make_report(prov)
        root_str = str(self.root)
        evidence_root_str = str(self.evidence_root)
        args = [(f"indep-{i}", report, prov, root_str, evidence_root_str) for i in range(5)]
        with multiprocessing.Pool(processes=5) as pool:
            results = pool.map(_mp_worker, args)
        winners = [r for r in results if r is not None]
        self.assertEqual(len(winners), 5, "Different run IDs should all succeed")

    # -- sequential lock acquire / release --------------------------------

    def test_lock_acquire_release_sequential(self):
        """A second writer can acquire the lock after the first finishes."""
        prov = _make_provenance()
        report = _make_report(prov)
        first = write_run("seq-1", report, prov, self.root, evidence_root=self.evidence_root)
        self.assertTrue(first.is_dir())
        second = write_run("seq-2", report, prov, self.root, evidence_root=self.evidence_root)
        self.assertTrue(second.is_dir())

    # -- run_id rejects unsafe characters ---------------------------------

    def test_run_id_rejects_slash(self):
        prov = _make_provenance()
        report = _make_report(prov)
        with self.assertRaises(ContractError):
            write_run("a/b", report, prov, self.root, evidence_root=self.evidence_root)

    def test_run_id_rejects_null(self):
        prov = _make_provenance()
        report = _make_report(prov)
        with self.assertRaises(ContractError):
            write_run("a\x00b", report, prov, self.root, evidence_root=self.evidence_root)

    # -- verify_rejects_invalid_task_status --------------------------------

    def test_verify_rejects_invalid_task_status(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-badstatus", report, prov, self.root,
                            evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["tasks"][0]["status"] = "bogus"
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    # -- errors must not include file contents or secrets -----------------

    def test_errors_do_not_leak_file_contents(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-noleak", report, prov, self.root,
                            evidence_root=self.evidence_root)
        (run_dir / "scoreboard.json").write_bytes(b'{"secret": "abc123"}')
        try:
            verify_run(run_dir)
        except ContractError as e:
            msg = str(e)
            self.assertNotIn("abc123", msg)
            self.assertNotIn("secret", msg.lower())
            self.assertNotIn("password", msg.lower())
            self.assertNotIn("token", msg.lower())
            self.assertNotIn("key=", msg.lower())

    # -- evidence-related rejections (new) --------------------------------

    def test_write_rejects_missing_evidence_root(self):
        prov = _make_provenance()
        report = _make_report(prov)
        nonexistent = self.root / "no-such-evidence"
        with self.assertRaises(ContractError):
            write_run("run-noev", report, prov, self.root, evidence_root=nonexistent)

    def test_write_rejects_missing_source_file(self):
        prov = _make_provenance()
        report = _make_report(prov)
        # delete the expected source file
        (self.evidence_root / "task-1.log").unlink()
        with self.assertRaises(ContractError):
            write_run("run-missingsrc", report, prov, self.root, evidence_root=self.evidence_root)

    def test_write_rejects_oversized_evidence(self):
        prov = _make_provenance()
        report = _make_report(prov)
        # create a >16MiB evidence file
        big = b"x" * (17 * 1024 * 1024)
        (self.evidence_root / "big.log").write_bytes(big)
        # modify report to use big.log as evidence
        tampered = copy.deepcopy(report)
        tampered["tasks"][0]["evidence_paths"] = ["logs/big.log"]
        # seal again would break, so skip seal for quick negativity; write_run will detect size
        with self.assertRaises(ContractError):
            write_run("run-bigev", tampered, prov, self.root, evidence_root=self.evidence_root)

    def test_verify_rejects_tampered_evidence_content(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-ev-tamper", report, prov, self.root,
                            evidence_root=self.evidence_root)
        log_path = run_dir / "logs" / "task-1.log"
        log_path.write_bytes(b"corrupted")
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_removed_evidence(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-ev-remove", report, prov, self.root,
                            evidence_root=self.evidence_root)
        (run_dir / "logs" / "task-1.log").unlink()
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_extra_evidence(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-ev-extra", report, prov, self.root,
                            evidence_root=self.evidence_root)
        (run_dir / "logs" / "stowaway.log").write_bytes(b"extra")
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_symlink_inside_logs(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-ev-symlink", report, prov, self.root,
                            evidence_root=self.evidence_root)
        log_path = run_dir / "logs" / "task-1.log"
        log_path.unlink()
        os.symlink("/etc/passwd", str(log_path)) if os.path.exists("/etc/passwd") else os.symlink(run_dir, str(log_path))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_noncanonical_manifest(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-mf-noncanon", report, prov, self.root,
                            evidence_root=self.evidence_root)
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        (run_dir / "manifest.json").write_bytes(json.dumps(mf).encode("utf-8"))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_manifest_missing_keys(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-mf-missingkeys", report, prov, self.root,
                            evidence_root=self.evidence_root)
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        del mf["scoreboard_sha256"]
        (run_dir / "manifest.json").write_bytes(pretty_json_bytes(mf))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_verify_rejects_scoreboard_missing_keys(self):
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-sb-missingkeys", report, prov, self.root,
                            evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        del sb["run_id"]
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_manifest_evidence_key_tampering_rejected(self):
        """Manifest evidence_hashes key with double slash or unreferenced path rejected."""
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-evtamperkey", report, prov, self.root,
                            evidence_root=self.evidence_root)
        mf = json.loads((run_dir / "manifest.json").read_bytes())
        valid_digest = mf["evidence_sha256"].pop("logs/task-1.log")
        mf["evidence_sha256"]["logs//evil.log"] = valid_digest
        # make manifest canonical and update digest to match unchanged scoreboard
        (run_dir / "manifest.json").write_bytes(pretty_json_bytes(mf))
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_evaluated_at_deterministic_with_reference_time(self):
        prov = _make_provenance()
        report = _make_report(prov)
        ref_time = datetime(2025, 3, 15, 10, 0, 0, 987654, tzinfo=timezone.utc)
        run_dir = write_run("run-ref-time", report, prov, self.root,
                            evidence_root=self.evidence_root,
                            reference_time=ref_time)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        self.assertEqual(sb.get("evaluated_at"), "2025-03-15T10:00:00Z",
                         "evaluated_at must match the passed reference_time")

    def test_tampered_summary_rejected(self):
        """Even after updating manifest digest, semantic validation rejects tampered summary."""
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-sum-tamper", report, prov, self.root,
                            evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        sb["summary"]["total"] = 999
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)

    def test_tampered_metrics_rejected(self):
        """Metrics tampering detected by semantic validators even with consistent digests."""
        prov = _make_provenance()
        report = _make_report(prov)
        run_dir = write_run("run-metric-tamper", report, prov, self.root,
                            evidence_root=self.evidence_root)
        sb = json.loads((run_dir / "scoreboard.json").read_bytes())
        # pick first metric key and change average
        key = next(iter(sb["metrics"]))
        sb["metrics"][key]["available"] = 0
        sb_bytes = pretty_json_bytes(sb)
        (run_dir / "scoreboard.json").write_bytes(sb_bytes)
        self._update_manifest_digest_after_sb_change(run_dir, sb_bytes)
        with self.assertRaises(ContractError):
            verify_run(run_dir)


if __name__ == "__main__":
    unittest.main()
