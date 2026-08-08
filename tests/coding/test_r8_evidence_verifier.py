#!/usr/bin/env python3
"""Deterministic tests for the R8 evidence verifier.

Covers positive receipt validation, negative receipt validation,
metadata validation, schema errors, cross-validation, and integration
through run_verifier.  Includes regression tests for wrong commit,
wrong run ID, archive mismatch, installed digest mismatch, receipt
byte tampering, symlink rejection, non-hex digest, and invented
safe-stop fields.
"""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"
import sys
sys.path.insert(0, str(HARNESS))

from r8_evidence_verifier import (  # noqa: E402
    verify_positive_receipt,
    verify_negative_receipt,
    verify_metadata,
    verify_distinct_episodes,
    run_verifier,
)


FIXTURE_DEVICE = "kuavo-mujoco-01"
FIXTURE_COMMIT = "a" * 40
FIXTURE_RUN_ID = "1234567890"


def _make_positive_receipt(**overrides) -> dict:
    base = {
        "schema_version": 1,
        "status": "REPORT_EVIDENCE_PASS",
        "episode_id": "ep-001",
        "settlement": "completed",
        "report_sha256": "e" * 64,
        "attempt_count": 3,
        "operation_ids": ["op-1", "op-2", "op-3"],
        "safe_stop": None,
        "runtime_facts": {
            "device": FIXTURE_DEVICE,
            "scene": "kuavo-mujoco/default-v40",
            "bridge_protocol_digest": "b" * 64,
            "skill_descriptor_digest": "c" * 64,
            "policy": {
                "provider": "test",
                "model": "test-model",
                "version": "1.0",
                "protocol": "1.0",
                "digest": "d" * 64,
            },
        },
    }
    base.update(overrides)
    return base


def _make_negative_receipt(**overrides) -> dict:
    base = {
        "schema_version": 1,
        "status": "REPORT_EVIDENCE_PASS",
        "episode_id": "ep-neg-001",
        "settlement": "failed",
        "report_sha256": "f" * 64,
        "attempt_count": 0,
        "operation_ids": [],
        "safe_stop": {
            "outcome": "succeeded",
            "attempted_after_attempt": 0,
        },
        "runtime_facts": {
            "device": FIXTURE_DEVICE,
            "scene": "kuavo-mujoco/default-v40",
            "bridge_protocol_digest": "b" * 64,
            "skill_descriptor_digest": "c" * 64,
            "policy": {
                "provider": "test",
                "model": "test-model",
                "version": "1.0",
                "protocol": "1.0",
                "digest": "d" * 64,
            },
        },
    }
    base.update(overrides)
    return base


def _make_metadata(**overrides) -> dict:
    base = {
        "schema_version": 1,
        "commit_sha": FIXTURE_COMMIT,
        "rc_archive_run_id": FIXTURE_RUN_ID,
        "rc_archive_sha256": "a" * 64,
        "installed_digest": "b" * 64,
        "positive_receipt_sha256": "c" * 64,
        "negative_receipt_sha256": "d" * 64,
        "device": FIXTURE_DEVICE,
        "generated_utc": datetime.now(timezone.utc).isoformat(),
    }
    base.update(overrides)
    return base


# ---------------------------------------------------------------------------
# verify_positive_receipt
# ---------------------------------------------------------------------------

class PositiveReceiptTest(unittest.TestCase):
    def test_valid_positive_passes(self):
        receipt = _make_positive_receipt()
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertEqual(reasons, [])

    def test_rejects_wrong_schema_version(self):
        receipt = _make_positive_receipt(schema_version=2)
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("schema_version must be 1" in r for r in reasons))

    def test_rejects_non_pass_status(self):
        receipt = _make_positive_receipt(status="REPORT_EVIDENCE_FAIL")
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("REPORT_EVIDENCE_PASS" in r for r in reasons))

    def test_rejects_non_completed_settlement(self):
        receipt = _make_positive_receipt(settlement="failed")
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("completed" in r for r in reasons))

    def test_rejects_zero_attempt_count(self):
        receipt = _make_positive_receipt(attempt_count=0)
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("attempt_count" in r for r in reasons))

    def test_rejects_empty_operation_ids(self):
        receipt = _make_positive_receipt(operation_ids=[])
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("operation_ids" in r for r in reasons))

    def test_rejects_operation_ids_count_mismatch(self):
        receipt = _make_positive_receipt(
            attempt_count=3, operation_ids=["op-1", "op-2"]
        )
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("operation_ids count" in r for r in reasons))

    def test_rejects_duplicate_operation_ids(self):
        receipt = _make_positive_receipt(operation_ids=["op-1", "op-1", "op-3"])
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("unique" in r for r in reasons))

    def test_rejects_nonempty_safe_stop_for_completed(self):
        receipt = _make_positive_receipt(safe_stop={"outcome": "succeeded"})
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("safe_stop" in r for r in reasons))

    def test_rejects_bad_report_sha256(self):
        receipt = _make_positive_receipt(report_sha256="short")
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("report_sha256" in r for r in reasons))

    def test_rejects_non_hex_report_sha256(self):
        receipt = _make_positive_receipt(report_sha256="g" * 64)
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("report_sha256" in r for r in reasons))

    def test_rejects_wrong_device(self):
        receipt = _make_positive_receipt()
        receipt["runtime_facts"]["device"] = "wrong-device"
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("device" in r for r in reasons))

    def test_rejects_non_dict(self):
        reasons = verify_positive_receipt([], FIXTURE_DEVICE)
        self.assertTrue(any("not a JSON object" in r for r in reasons))

    def test_rejects_empty_operation_id_string(self):
        receipt = _make_positive_receipt(
            attempt_count=1, operation_ids=[""]
        )
        reasons = verify_positive_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("non-empty string" in r for r in reasons))


# ---------------------------------------------------------------------------
# verify_negative_receipt
# ---------------------------------------------------------------------------

class NegativeReceiptTest(unittest.TestCase):
    def test_valid_negative_zero_attempts_passes(self):
        receipt = _make_negative_receipt()
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertEqual(reasons, [])

    def test_valid_negative_with_trigger_passes(self):
        receipt = _make_negative_receipt(
            attempt_count=1,
            operation_ids=["op-neg-1"],
            safe_stop={
                "outcome": "succeeded",
                "attempted_after_attempt": 1,
                "trigger": "SafetyFallback",
            },
        )
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertEqual(reasons, [])

    def test_rejects_invented_safe_stop_triggered_field(self):
        receipt = _make_negative_receipt()
        receipt["safe_stop"]["triggered"] = True
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertEqual(reasons, [])

    def test_rejects_wrong_schema_version(self):
        receipt = _make_negative_receipt(schema_version=2)
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("schema_version must be 1" in r for r in reasons))

    def test_rejects_non_pass_status(self):
        receipt = _make_negative_receipt(status="REPORT_EVIDENCE_FAIL")
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("REPORT_EVIDENCE_PASS" in r for r in reasons))

    def test_rejects_non_failed_settlement(self):
        receipt = _make_negative_receipt(settlement="completed")
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("failed" in r for r in reasons))

    def test_rejects_missing_safe_stop(self):
        receipt = _make_negative_receipt(safe_stop=None)
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("safe_stop" in r for r in reasons))

    def test_rejects_safe_stop_not_succeeded(self):
        receipt = _make_negative_receipt()
        receipt["safe_stop"]["outcome"] = "failed"
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("safe_stop.outcome" in r for r in reasons))

    def test_rejects_attempted_after_attempt_mismatch(self):
        receipt = _make_negative_receipt(
            attempt_count=2,
            operation_ids=["a", "b"],
            safe_stop={
                "outcome": "succeeded",
                "attempted_after_attempt": 1,
            },
        )
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("attempted_after_attempt" in r for r in reasons))

    def test_rejects_wrong_device(self):
        receipt = _make_negative_receipt()
        receipt["runtime_facts"]["device"] = "wrong-device"
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("device" in r for r in reasons))

    def test_rejects_negative_attempt_count(self):
        receipt = _make_negative_receipt(attempt_count=-1)
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("attempt_count" in r for r in reasons))

    def test_rejects_operation_ids_mismatch(self):
        receipt = _make_negative_receipt(
            attempt_count=1, operation_ids=["a", "b"]
        )
        reasons = verify_negative_receipt(receipt, FIXTURE_DEVICE)
        self.assertTrue(any("operation_ids count" in r for r in reasons))


# ---------------------------------------------------------------------------
# verify_distinct_episodes
# ---------------------------------------------------------------------------

class DistinctEpisodesTest(unittest.TestCase):
    def test_distinct_passes(self):
        reasons = verify_distinct_episodes(
            _make_positive_receipt(episode_id="ep-a", report_sha256="a" * 64),
            _make_negative_receipt(episode_id="ep-b", report_sha256="b" * 64),
        )
        self.assertEqual(reasons, [])

    def test_same_episode_id_fails(self):
        reasons = verify_distinct_episodes(
            _make_positive_receipt(episode_id="ep-same"),
            _make_negative_receipt(episode_id="ep-same"),
        )
        self.assertTrue(any("episode_id" in r for r in reasons))

    def test_same_report_sha256_fails(self):
        digest = "a" * 64
        reasons = verify_distinct_episodes(
            _make_positive_receipt(
                episode_id="ep-a", report_sha256=digest
            ),
            _make_negative_receipt(
                episode_id="ep-b", report_sha256=digest
            ),
        )
        self.assertTrue(any("report_sha256" in r for r in reasons))


# ---------------------------------------------------------------------------
# verify_metadata
# ---------------------------------------------------------------------------

class MetadataTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="r8m_")
        self.tmp_dir = Path(self._tmp.name)
        self._rc_archive = self.tmp_dir / "rc.tar.gz"
        self._rc_archive.write_bytes(b"fake archive content for digest")
        self._archive_digest = hashlib.sha256(
            self._rc_archive.read_bytes()
        ).hexdigest()
        self._pos = self.tmp_dir / "pos.json"
        self._pos.write_text(json.dumps(_make_positive_receipt()) + "\n")
        self._pos_digest = hashlib.sha256(self._pos.read_bytes()).hexdigest()
        self._neg = self.tmp_dir / "neg.json"
        self._neg.write_text(json.dumps(_make_negative_receipt()) + "\n")
        self._neg_digest = hashlib.sha256(self._neg.read_bytes()).hexdigest()

    def tearDown(self):
        self._tmp.cleanup()

    def test_valid_metadata_passes(self):
        meta = _make_metadata(
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta,
            FIXTURE_COMMIT,
            FIXTURE_RUN_ID,
            self._rc_archive,
            meta["installed_digest"],
            FIXTURE_DEVICE,
            self._pos,
            self._neg,
        )
        self.assertEqual(reasons, [])

    def test_wrong_commit_fails(self):
        meta = _make_metadata(
            commit_sha="f" * 40,
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("commit_sha" in r for r in reasons))

    def test_wrong_run_id_fails(self):
        meta = _make_metadata(
            rc_archive_run_id="9999999999",
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("rc_archive_run_id" in r for r in reasons))

    def test_archive_digest_mismatch_fails(self):
        meta = _make_metadata(
            rc_archive_sha256="f" * 64,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("rc_archive_sha256" in r for r in reasons))

    def test_installed_digest_mismatch_fails(self):
        meta = _make_metadata(
            installed_digest="f" * 64,
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, "b" * 64,
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("installed_digest" in r for r in reasons))

    def test_missing_generated_utc_fails(self):
        meta = _make_metadata(
            generated_utc="",
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("generated_utc" in r for r in reasons))

    def test_non_utc_generated_utc_fails(self):
        meta = _make_metadata(
            generated_utc="2026-08-07T12:00:00",
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("generated_utc" in r for r in reasons))

    def test_positive_receipt_digest_mismatch_fails(self):
        meta = _make_metadata(
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256="f" * 64,
            negative_receipt_sha256=self._neg_digest,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("positive_receipt_sha256" in r for r in reasons))

    def test_negative_receipt_digest_mismatch_fails(self):
        meta = _make_metadata(
            rc_archive_sha256=self._archive_digest,
            positive_receipt_sha256=self._pos_digest,
            negative_receipt_sha256="f" * 64,
        )
        reasons = verify_metadata(
            meta, FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, meta["installed_digest"],
            FIXTURE_DEVICE, self._pos, self._neg,
        )
        self.assertTrue(any("negative_receipt_sha256" in r for r in reasons))


# ---------------------------------------------------------------------------
# run_verifier integration
# ---------------------------------------------------------------------------

class RunVerifierTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="r8v_")
        self.tmp_dir = Path(self._tmp.name)
        self._rc_archive = self.tmp_dir / "rc.tar.gz"
        self._rc_archive.write_bytes(b"fake archive content")
        self._archive_digest = hashlib.sha256(
            self._rc_archive.read_bytes()
        ).hexdigest()

    def tearDown(self):
        self._tmp.cleanup()

    def _write_json(self, name: str, data: dict) -> Path:
        path = self.tmp_dir / name
        path.write_text(json.dumps(data, sort_keys=True) + "\n")
        return path

    def _file_digest(self, path: Path) -> str:
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def test_both_valid_passes(self):
        pos = self._write_json("positive.json", _make_positive_receipt())
        neg = self._write_json("negative.json", _make_negative_receipt())
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 0)

    def test_missing_metadata_fails(self):
        pos = self._write_json("positive.json", _make_positive_receipt())
        neg = self._write_json("negative.json", _make_negative_receipt())
        rc = run_verifier(
            self.tmp_dir / "nonexistent.json", pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, "b" * 64, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_missing_positive_fails(self):
        neg = self._write_json("negative.json", _make_negative_receipt())
        installed_digest = "b" * 64
        pos_path = self.tmp_dir / "positive.json"
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256="c" * 64,
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, pos_path, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_missing_negative_fails(self):
        pos = self._write_json("positive.json", _make_positive_receipt())
        installed_digest = "b" * 64
        neg_path = self.tmp_dir / "negative.json"
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256="d" * 64,
        ))
        rc = run_verifier(
            meta, pos, neg_path,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_invalid_positive_fails(self):
        pos = self._write_json("positive.json", _make_positive_receipt(settlement="failed"))
        neg = self._write_json("negative.json", _make_negative_receipt())
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_invalid_negative_fails(self):
        pos = self._write_json("positive.json", _make_positive_receipt())
        neg = self._write_json("negative.json", _make_negative_receipt(safe_stop=None))
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_malformed_json_fails(self):
        bad = self.tmp_dir / "positive.json"
        bad.write_text("not json")
        neg = self._write_json("negative.json", _make_negative_receipt())
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256="c" * 64,
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, bad, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_symlink_receipt_fails(self):
        pos = self._write_json("real-pos.json", _make_positive_receipt())
        neg = self._write_json("negative.json", _make_negative_receipt())
        symlink_pos = self.tmp_dir / "positive.json"
        symlink_pos.symlink_to("real-pos.json")
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256="c" * 64,
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, symlink_pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_receipt_byte_tampering_fails(self):
        pos = self._write_json("positive.json", _make_positive_receipt())
        neg = self._write_json("negative.json", _make_negative_receipt())
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256=self._file_digest(neg),
        ))
        # Tamper with positive receipt bytes after metadata was computed
        pos.write_text(pos.read_text() + "extra")
        rc = run_verifier(
            meta, pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)

    def test_same_episode_ids_fails(self):
        same_id = "ep-same"
        pos = self._write_json("positive.json", _make_positive_receipt(
            episode_id=same_id, report_sha256="a" * 64,
        ))
        neg = self._write_json("negative.json", _make_negative_receipt(
            episode_id=same_id, report_sha256="b" * 64,
        ))
        installed_digest = "b" * 64
        meta = self._write_json("metadata.json", _make_metadata(
            rc_archive_sha256=self._archive_digest,
            installed_digest=installed_digest,
            positive_receipt_sha256=self._file_digest(pos),
            negative_receipt_sha256=self._file_digest(neg),
        ))
        rc = run_verifier(
            meta, pos, neg,
            FIXTURE_COMMIT, FIXTURE_RUN_ID,
            self._rc_archive, installed_digest, FIXTURE_DEVICE,
        )
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
