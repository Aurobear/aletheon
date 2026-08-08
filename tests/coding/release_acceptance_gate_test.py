#!/usr/bin/env python3
"""Deterministic tests for release_acceptance_gate.

Covers every success and binding/tamper failure required by the release-gate
contract, plus static workflow assertions.
"""

from __future__ import annotations

import hashlib
import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
HARNESS = HERE / "harness"
sys.path.insert(0, str(HARNESS))

from acceptance_artifacts import pretty_json_bytes, write_run  # noqa: E402
from acceptance_contract import ContractError  # noqa: E402
from receipt import METRIC_KEYS, seal  # noqa: E402
from release_acceptance_gate import (  # noqa: E402
    _compute_sha256_from_tar_member,
    _find_and_hash_aletheon_binary,
    run_gate,
)


# ---------------------------------------------------------------------------
# deterministic fixtures
# ---------------------------------------------------------------------------

FIXTURE_BINARY = b"aletheon-x86_64-release-binary-v1.0.0\n"
FIXTURE_BINARY_DIGEST = hashlib.sha256(FIXTURE_BINARY).hexdigest()
FIXTURE_COMMIT = "a" * 40
FIXTURE_DIFFERENT_COMMIT = "b" * 40
FIXTURE_DIFFERENT_BINARY = b"tampered-binary-content!!!!\n"
FIXTURE_DIFFERENT_DIGEST = hashlib.sha256(FIXTURE_DIFFERENT_BINARY).hexdigest()


def _sha256_hex(s: str) -> str:
    return hashlib.sha256(s.encode()).hexdigest()


# ---------------------------------------------------------------------------
# helpers: build a full 20-task passing provenance + report
# ---------------------------------------------------------------------------

def _make_18pass_2fail_report(provenance: dict) -> dict:
    """Build a valid sealed report with 18 passed + 2 failed tasks.

    The gate_verdict threshold is ``passed_count < 19``, so 18 passed
    produces a genuinely failed gate with reason ``insufficient_passed``.
    """
    installed_sha = provenance["installed_artifact"]["sha256"]
    fixture_digest = provenance["fixture_digest"]
    gen_id = provenance["generation_id"]

    metric_template = {
        "available": 20, "unavailable": 0,
        "average": 0.0, "p50": 0, "p95": 0,
    }
    metrics = {key: dict(metric_template) for key in METRIC_KEYS}

    tasks = []
    for i in range(1, 21):
        tid = f"task-{i:02d}"
        is_passing = i <= 18
        tasks.append({
            "task_id": tid,
            "category": "behavioral_bugfix",
            "receipt_valid": True,
            "execution_present": True,
            "outcome_passed": is_passing,
            "failure_class": "none" if is_passing else "assertion_error",
            "reasons": [] if is_passing else ["test failure"],
            "scope_violation_count": 0,
            "resource_leak_count": 0,
            "terminal_settlement_count": 1,
            "retry_count": 0,
            "started_at": f"2025-01-01T00:{i-1:02d}:00Z",
            "ended_at": f"2025-01-01T00:{i-1:02d}:30Z",
            "exit_code": 0 if is_passing else 1,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": [f"logs/task-{i:02d}.log"],
            "generation_id": gen_id,
            "p0": False,
            "waiver": None,
        })

    by_cat = {"behavioral_bugfix": {"total": 20, "passed": 18, "failed": 2}}

    return seal({
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
            "total": 20,
            "benchmark_outcomes_passed": 18,
            "benchmark_outcomes_failed": 2,
            "completed_engineering_tasks": 0,
            "expected_non_success_outcomes": 0,
            "validation_pass_rate": 0.9,
            "evidence_complete_success_rate": 1.0,
            "false_success_count": 0,
            "scope_violation_count": 0,
            "leaked_resource_count": 0,
            "by_category": by_cat,
            "by_failure_class": {
                "none": 18,
                "assertion_error": 2,
                "timeout": 0,
                "terminal_settlement_timeout": 0,
                "panic": 0,
                "resource_leak": 0,
            },
            "by_observed_terminal": {
                "verified": 20,
                "killed": 0,
                "unobserved": 0,
                "timeout": 0,
            },
        },
        "metrics": metrics,
        "tasks": tasks,
    })


def _write_run_18pass_2fail(
    root: Path,
    evidence_root: Path,
    run_id: str = "v0.1.0",
    installed_sha: str = FIXTURE_BINARY_DIGEST,
    commit_sha: str = FIXTURE_COMMIT,
) -> Path:
    """Write a valid 18-pass/2-fail acceptance run and return its path."""
    prov = _make_20task_provenance(
        installed_sha=installed_sha, commit_sha=commit_sha
    )
    report = _make_18pass_2fail_report(prov)
    gen_id = prov["generation_id"]
    for i in range(1, 21):
        (evidence_root / f"task-{i:02d}.log").write_bytes(
            f"log for task-{i:02d}\n".encode()
        )
    return write_run(
        run_id, report, prov, root, evidence_root=evidence_root
    )


def _make_20task_provenance(
    installed_sha: str = FIXTURE_BINARY_DIGEST,
    commit_sha: str = FIXTURE_COMMIT,
) -> dict:
    fixture_digest = _sha256_hex("fixture-20task")
    return {
        "repo": {"sha": commit_sha, "dirty": False},
        "build": {"profile": "release", "features": []},
        "environment_level": "installed",
        "installed_artifact": {
            "path": "/usr/bin/aletheon",
            "sha256": installed_sha,
        },
        "client": {"version": "1.0.0", "protocol_version": "1"},
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
        "generation_id": "gen.release-test.1",
    }


def _make_20task_report(provenance: dict) -> dict:
    installed_sha = provenance["installed_artifact"]["sha256"]
    fixture_digest = provenance["fixture_digest"]
    gen_id = provenance["generation_id"]

    metric_template = {
        "available": 20, "unavailable": 0,
        "average": 0.0, "p50": 0, "p95": 0,
    }
    metrics = {key: dict(metric_template) for key in METRIC_KEYS}

    tasks = []
    for i in range(1, 21):
        tid = f"task-{i:02d}"
        tasks.append({
            "task_id": tid,
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
            "started_at": f"2025-01-01T00:{i-1:02d}:00Z",
            "ended_at": f"2025-01-01T00:{i-1:02d}:30Z",
            "exit_code": 0,
            "expected_terminal": "verified",
            "observed_terminal": "verified",
            "evidence_paths": [f"logs/task-{i:02d}.log"],
            "generation_id": gen_id,
            "p0": False,
            "waiver": None,
        })

    # build the category dict for 20 tasks all in behavioral_bugfix
    by_cat = {"behavioral_bugfix": {"total": 20, "passed": 20, "failed": 0}}

    return seal({
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
            "total": 20,
            "benchmark_outcomes_passed": 20,
            "benchmark_outcomes_failed": 0,
            "completed_engineering_tasks": 0,
            "expected_non_success_outcomes": 0,
            "validation_pass_rate": 1.0,
            "evidence_complete_success_rate": 1.0,
            "false_success_count": 0,
            "scope_violation_count": 0,
            "leaked_resource_count": 0,
            "by_category": by_cat,
            "by_failure_class": {
                "none": 20,
                "assertion_error": 0,
                "timeout": 0,
                "terminal_settlement_timeout": 0,
                "panic": 0,
                "resource_leak": 0,
            },
            "by_observed_terminal": {
                "verified": 20,
                "killed": 0,
                "unobserved": 0,
                "timeout": 0,
            },
        },
        "metrics": metrics,
        "tasks": tasks,
    })


def _make_tar_gz(
    binary_content: bytes,
    archive_dir: str = "aletheon-v0.1.0-x86_64-unknown-linux-gnu",
) -> bytes:
    """Create an in-memory tar.gz containing a single regular file
    ``<archive_dir>/aletheon`` with *binary_content*."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        info = tarfile.TarInfo(name=f"{archive_dir}/aletheon")
        info.size = len(binary_content)
        info.mode = 0o755
        info.type = tarfile.REGTYPE
        tf.addfile(info, io.BytesIO(binary_content))
    return buf.getvalue()


def _write_run_20task(
    root: Path,
    evidence_root: Path,
    run_id: str = "v0.1.0",
    installed_sha: str = FIXTURE_BINARY_DIGEST,
    commit_sha: str = FIXTURE_COMMIT,
    dirty: bool = False,
) -> Path:
    """Write a valid 20-task acceptance run and return its path."""
    prov = _make_20task_provenance(
        installed_sha=installed_sha, commit_sha=commit_sha
    )
    prov["repo"]["dirty"] = dirty
    report = _make_20task_report(prov)
    # create per-task evidence files
    gen_id = prov["generation_id"]
    for i in range(1, 21):
        (evidence_root / f"task-{i:02d}.log").write_bytes(
            f"log for task-{i:02d}\n".encode()
        )
    return write_run(
        run_id, report, prov, root, evidence_root=evidence_root
    )


# ---------------------------------------------------------------------------
# tests
# ---------------------------------------------------------------------------

class ReleaseAcceptanceGateTest(unittest.TestCase):
    """Tests for run_gate covering every success and binding/tamper failure."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="rel_gate_")
        self.root = Path(self._tmp.name)
        self._ev_tmp = tempfile.TemporaryDirectory(prefix="rel_ev_")
        self.evidence_root = Path(self._ev_tmp.name)
        self._archive_tmp = tempfile.TemporaryDirectory(prefix="rel_arc_")
        self.archive_dir = Path(self._archive_tmp.name)

    def tearDown(self):
        self._tmp.cleanup()
        self._ev_tmp.cleanup()
        self._archive_tmp.cleanup()

    # ------------------------------------------------------------------
    # helpers
    # ------------------------------------------------------------------

    def _archive_path(self, name: str = "aletheon-v0.1.0-x86_64-unknown-linux-gnu.tar.gz") -> Path:
        return self.archive_dir / name

    def _write_archive(
        self,
        binary: bytes = FIXTURE_BINARY,
        name: str = "aletheon-v0.1.0-x86_64-unknown-linux-gnu.tar.gz",
    ) -> Path:
        path = self._archive_path(name)
        path.write_bytes(_make_tar_gz(binary))
        return path

    # ------------------------------------------------------------------
    # success
    # ------------------------------------------------------------------

    def test_gate_passes_when_all_bindings_match(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 0)

    # ------------------------------------------------------------------
    # missing / malformed run directory
    # ------------------------------------------------------------------

    def test_fails_when_run_dir_missing(self):
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(self.root / "nonexistent", FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    def test_fails_when_run_dir_is_symlink(self):
        real_dir = _write_run_20task(self.root, self.evidence_root)
        link = self.root / "link_run"
        link.symlink_to(real_dir)
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(link, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    def test_fails_when_run_dir_is_file(self):
        f = self.root / "not_a_dir"
        f.write_text("not a directory")
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(f, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    # ------------------------------------------------------------------
    # failed gate (scoreboard has gate.status != 'passed')
    # ------------------------------------------------------------------

    def test_fails_when_gate_is_genuinely_failed(self):
        """A valid 18-pass/2-fail run where verify_run accepts but
        gate.status is 'failed' so run_gate must reject."""
        run_dir = _write_run_18pass_2fail(self.root, self.evidence_root)
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    def test_fails_when_scoreboard_is_tampered(self):
        """Tampering the scoreboard gate without updating tasks causes
        verify_run to detect a gate-reconstruction mismatch."""
        run_dir = _write_run_20task(self.root, self.evidence_root)
        sb_path = run_dir / "scoreboard.json"
        sb = json.loads(sb_path.read_bytes())
        sb["gate"] = {"status": "failed", "reasons": ["task_count"]}
        sb_path.write_bytes(pretty_json_bytes(sb))
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    # ------------------------------------------------------------------
    # commit mismatch
    # ------------------------------------------------------------------

    def test_fails_when_commit_does_not_match(self):
        run_dir = _write_run_20task(
            self.root, self.evidence_root, commit_sha=FIXTURE_COMMIT
        )
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_DIFFERENT_COMMIT, archive)
        self.assertEqual(rc, 1)

    def test_fails_when_acceptance_tree_was_dirty(self):
        run_dir = _write_run_20task(
            self.root, self.evidence_root, dirty=True
        )
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    # ------------------------------------------------------------------
    # binary digest mismatch
    # ------------------------------------------------------------------

    def test_fails_when_binary_digest_differs(self):
        run_dir = _write_run_20task(
            self.root, self.evidence_root,
            installed_sha=FIXTURE_BINARY_DIGEST,
        )
        archive = self._write_archive(FIXTURE_DIFFERENT_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    # ------------------------------------------------------------------
    # malformed / missing archive
    # ------------------------------------------------------------------

    def test_fails_when_archive_missing(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        rc = run_gate(
            run_dir, FIXTURE_COMMIT,
            self._archive_path("nonexistent.tar.gz"),
        )
        self.assertEqual(rc, 1)

    def test_fails_when_archive_is_not_tar_gz(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        bad = self._archive_path("bad.tar.gz")
        bad.write_bytes(b"not a valid gzip stream!!!!!")
        rc = run_gate(run_dir, FIXTURE_COMMIT, bad)
        self.assertEqual(rc, 1)

    def test_fails_when_archive_has_no_aletheon_member(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        # create archive without any aletheon member
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            info = tarfile.TarInfo(name="staging/other-binary")
            info.size = 4
            info.type = tarfile.REGTYPE
            tf.addfile(info, io.BytesIO(b"data"))
        path = self._archive_path("no-aletheon.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    def test_fails_when_archive_has_multiple_aletheon_members(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            for name in ("dir-a/aletheon", "dir-b/aletheon"):
                info = tarfile.TarInfo(name=name)
                info.size = len(FIXTURE_BINARY)
                info.type = tarfile.REGTYPE
                tf.addfile(info, io.BytesIO(FIXTURE_BINARY))
        path = self._archive_path("dupe-aletheon.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    def test_fails_when_aletheon_member_is_symlink(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            info = tarfile.TarInfo(name="staging/aletheon")
            info.type = tarfile.SYMTYPE
            info.linkname = "/usr/bin/aletheon"
            tf.addfile(info)
        path = self._archive_path("symlink-aletheon.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    def test_fails_when_archive_has_regular_and_symlink_aletheon(self):
        """One regular + one symlink both ending with /aletheon: must
        be rejected as duplicates before checking isfile()."""
        run_dir = _write_run_20task(self.root, self.evidence_root)
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            info = tarfile.TarInfo(name="dir-a/aletheon")
            info.size = len(FIXTURE_BINARY)
            info.type = tarfile.REGTYPE
            info.mode = 0o755
            tf.addfile(info, io.BytesIO(FIXTURE_BINARY))
            info2 = tarfile.TarInfo(name="dir-b/aletheon")
            info2.type = tarfile.SYMTYPE
            info2.linkname = "dir-a/aletheon"
            tf.addfile(info2)
        path = self._archive_path("mixed-aletheon.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    def test_fails_when_archive_has_corrupted_tar_data(self):
        """Valid gzip stream wrapping corrupt tar data must produce
        a concise ContractError, not an unhandled traceback."""
        import gzip
        run_dir = _write_run_20task(self.root, self.evidence_root)
        buf = io.BytesIO()
        with gzip.GzipFile(fileobj=buf, mode="wb") as gz:
            gz.write(b"not valid tar content!!!!!!")
        path = self._archive_path("corrupt-tar.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    def test_fails_when_aletheon_member_is_directory(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            info = tarfile.TarInfo(name="staging/aletheon")
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            tf.addfile(info)
        path = self._archive_path("dir-aletheon.tar.gz")
        path.write_bytes(buf.getvalue())
        rc = run_gate(run_dir, FIXTURE_COMMIT, path)
        self.assertEqual(rc, 1)

    # ------------------------------------------------------------------
    # tampered scoreboard / manifest (verify_run should catch these)
    # ------------------------------------------------------------------

    def test_fails_when_manifest_is_missing(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        (run_dir / "manifest.json").unlink()
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)

    def test_fails_when_scoreboard_is_missing(self):
        run_dir = _write_run_20task(self.root, self.evidence_root)
        (run_dir / "scoreboard.json").unlink()
        archive = self._write_archive(FIXTURE_BINARY)
        rc = run_gate(run_dir, FIXTURE_COMMIT, archive)
        self.assertEqual(rc, 1)


# ---------------------------------------------------------------------------
# unit tests for internal helpers
# ---------------------------------------------------------------------------

class TarMemberHashTest(unittest.TestCase):
    """Tests for _find_and_hash_aletheon_binary and _compute_sha256_from_tar_member."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="tar_hash_")
        self.tmp_dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def _write_tar(self, name: str, members: list[tuple[str, bytes, int]]) -> Path:
        """Write a tar.gz with *members* (name, content, type)."""
        path = self.tmp_dir / name
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tf:
            for mname, mdata, mtype in members:
                info = tarfile.TarInfo(name=mname)
                info.size = len(mdata)
                info.type = mtype
                if mtype == tarfile.REGTYPE:
                    info.mode = 0o755
                elif mtype == tarfile.DIRTYPE:
                    info.mode = 0o755
                elif mtype == tarfile.SYMTYPE:
                    info.linkname = "/dev/null"
                tf.addfile(info, io.BytesIO(mdata) if mdata else io.BytesIO())
        path.write_bytes(buf.getvalue())
        return path

    def test_hashes_correct_binary(self):
        path = self._write_tar("good.tar.gz", [
            ("pkg-x86_64/aletheon", FIXTURE_BINARY, tarfile.REGTYPE),
        ])
        digest = _find_and_hash_aletheon_binary(path)
        self.assertEqual(digest, FIXTURE_BINARY_DIGEST)

    def test_skips_non_matching_members(self):
        path = self._write_tar("mixed.tar.gz", [
            ("pkg-x86_64/README.md", b"readme", tarfile.REGTYPE),
            ("pkg-x86_64/aletheon", FIXTURE_BINARY, tarfile.REGTYPE),
            ("pkg-x86_64/LICENSE", b"license", tarfile.REGTYPE),
        ])
        digest = _find_and_hash_aletheon_binary(path)
        self.assertEqual(digest, FIXTURE_BINARY_DIGEST)

    def test_raises_on_missing_member(self):
        path = self._write_tar("no-bin.tar.gz", [
            ("pkg-x86_64/other", b"x", tarfile.REGTYPE),
        ])
        with self.assertRaises(ContractError):
            _find_and_hash_aletheon_binary(path)

    def test_raises_on_duplicate_member(self):
        path = self._write_tar("dup.tar.gz", [
            ("a/aletheon", FIXTURE_BINARY, tarfile.REGTYPE),
            ("b/aletheon", FIXTURE_BINARY, tarfile.REGTYPE),
        ])
        with self.assertRaises(ContractError):
            _find_and_hash_aletheon_binary(path)

    def test_raises_on_symlink_member(self):
        path = self._write_tar("sym.tar.gz", [
            ("pkg-x86_64/aletheon", b"", tarfile.SYMTYPE),
        ])
        with self.assertRaises(ContractError):
            _find_and_hash_aletheon_binary(path)

    def test_raises_on_directory_member(self):
        path = self._write_tar("dir.tar.gz", [
            ("pkg-x86_64/aletheon", b"", tarfile.DIRTYPE),
        ])
        with self.assertRaises(ContractError):
            _find_and_hash_aletheon_binary(path)

    def test_raises_on_non_archive_file(self):
        path = self.tmp_dir / "not-an-archive"
        path.write_text("hello")
        with self.assertRaises(ContractError):
            _find_and_hash_aletheon_binary(path)


if __name__ == "__main__":
    unittest.main()
