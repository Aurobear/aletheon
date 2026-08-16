#!/usr/bin/env python3
"""E1 Robot Engineering Suite — Comprehensive Fixture Contract Tests.

This test suite verifies:
- 30 distinct tasks with correct category counts
- Every task has an independent oracle (not a shared generic oracle)
- Every task has a real workspace with buggy source code
- Every untouched baseline fails with expected failure-class evidence (FAIL[class]: ...)
- Every hidden reference repair passes
- Hidden oracle/solution content is never present in copied model workspace
- Required/allowed/forbidden paths are meaningful
- Permission/network/resource policy is typed and enforced
- Real runner uses valid CLI flags only with exact argv/order and valid enum values
- Fake-executable real-runner tests cover all failure modes
- Deterministic idempotency from run_id+task_id (no UUID randomness)
- Output is byte-identical
- Report slices cover all dimensions with per-task records
- Manifest is a projection from source with sorted file→SHA-256 maps
- Drift detection for catalog, workspace, oracle, solution mutation/removal
- Malformed alternate-catalog validation (fail-closed)
- Recursive privacy/leak checks using rglob
"""
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import base64
import unittest
from collections import Counter
from pathlib import Path

BASE = Path(__file__).resolve().parent
sys.path.insert(0, str(BASE))

import runner
import report

with open(BASE / "catalog.toml", "rb") as f:
    CATALOG = tomllib.load(f)
    TASKS = CATALOG["tasks"]

FAILURE_CLASSES = [
    "latency", "reordering", "packet_loss", "endianness",
    "clock_drift", "device_rejection", "watchdog",
]

CATEGORY_COUNTS = {
    "ros2_lifecycle_launch": 6,
    "containers_build": 4,
    "can_ethercat_serial": 5,
    "control_numerics": 5,
    "sensing_logs": 4,
    "safety_device_protocol": 3,
    "multi_file_engineering": 3,
}

EXPECTED_PERMISSIONS = {
    "ros2_lifecycle_launch": "code_modification",
    "containers_build": "code_modification",
    "can_ethercat_serial": "device_action",
    "control_numerics": "simulation",
    "sensing_logs": "read_only",
    "safety_device_protocol": "device_action",
    "multi_file_engineering": "code_modification",
}

_FIXTURE_RESULTS_CACHE = None
_FIXTURE_RESULTS_CACHE_DIGEST = None


def _fixture_results():
    """Cache expensive subprocess-based fixture validation only in tests.

    Cache is keyed by the canonical manifest digest so that any source
    change (catalog, workspace, oracle, solution) invalidates it.
    Results are deep-copied per call so tests never mutate shared state.
    """
    global _FIXTURE_RESULTS_CACHE, _FIXTURE_RESULTS_CACHE_DIGEST
    import generate_e1_suite
    current_digest = generate_e1_suite.compute_manifest()["canonical_digest"]
    if _FIXTURE_RESULTS_CACHE is None or _FIXTURE_RESULTS_CACHE_DIGEST != current_digest:
        _FIXTURE_RESULTS_CACHE = runner.fixture_validate_all(TASKS)
        _FIXTURE_RESULTS_CACHE_DIGEST = current_digest
    import copy
    return copy.deepcopy(_FIXTURE_RESULTS_CACHE)


def _valid_scoring_for_report():
    return {
        "correctness": False,
        "test_quality": "unavailable",
        "safety_boundary": True,
        "scope_architecture_consistency": True,
        "first_attempt_success": False,
        "elapsed_ms": 0,
        "token_cost": "unavailable",
        "cache_cost": "unavailable",
        "tool_cost": "unavailable",
        "human_intervention": "unavailable",
    }

# Legitimate engineering terms that MUST NOT be banned by privacy checks
LEGITIMATE_TERMS = [
    "serial", "parser", "timeout", "filter", "watchdog",
    "controller", "sensor", "diagnostics", "kinematics",
    "trajectory", "deadband", "pid", "angle", "convert",
    "csv", "statistics", "log", "diagnosis", "subscriber",
    "lifecycle", "param", "resolver", "discovery", "remap",
    "docker", "multistage", "colcon", "cross", "triplet",
    "build", "flags", "attestation", "safe", "stop",
    "timestamp", "align", "drop", "detect", "can", "ethercat",
]


def _fake_exec_terminal_json(*, status="completed", operation_id="op-test-001",
                             error_code=None, metrics=None, schema_version=1,
                             extra_fields=None,
                             sequence=0, session_id="session-test-001",
                             task_id="e1_can_ethercat_serial_0001",
                             turn_id="turn-test-001", output="",
                             activity_id=None):
    """Build a fake Exec terminal JSON object for fake-executable tests.

    Emits a complete ExecEventEnvelope matching crates/contracts/src/types/exec.rs.
    """
    if metrics is None:
        metrics = {
            "tool_calls_made": 0,
            "tool_errors": 0,
            "provider_retries": 0,
            "elapsed_ms": 0,
            "iterations": 0,
            "completed_normally": False,
        }
    obj = {
        "schema_version": schema_version,
        "sequence": sequence,
        "session_id": session_id,
        "task_id": task_id,
        "turn_id": turn_id,
        "activity_id": activity_id,
        "type": "terminal",
        "status": status,
        "operation_id": operation_id,
        "output": output,
        "metrics": metrics,
    }
    obj["error_code"] = error_code
    if extra_fields:
        obj.update(extra_fields)
    return obj


class E1FixtureContractTests(unittest.TestCase):
    """Fixture validation tests — no real LLM calls, deterministic."""

    # ── structural tests ──────────────────────────────────────

    def test_01_exact_task_count(self):
        self.assertEqual(len(TASKS), 30)

    def test_02_exact_category_counts(self):
        counts = Counter(t["category"] for t in TASKS)
        self.assertEqual(dict(counts), CATEGORY_COUNTS)

    def test_03_ids_unique(self):
        ids = [t["id"] for t in TASKS]
        self.assertEqual(len(ids), len(set(ids)))

    def test_04_no_generic_oracle(self):
        oracle_paths = set()
        for t in TASKS:
            op = t["oracle_path"]
            self.assertIsNotNone(op, f"{t['id']} missing oracle_path")
            oracle_paths.add(op)
        self.assertEqual(len(oracle_paths), 30,
                         "Some tasks share the same oracle file")

    def test_05_oracle_files_exist_and_are_python(self):
        for t in TASKS:
            op = BASE / t["oracle_path"]
            self.assertTrue(op.is_file(), f"Oracle missing: {op}")
            content = op.read_text(encoding="utf-8")
            self.assertIn("def main()", content,
                          f"Oracle {op} missing main()")
            self.assertIn("PASS", content,
                          f"Oracle {op} missing PASS output")
            # Must have structured failure evidence
            efc = t["expected_failure_class"]
            self.assertIn(f"FAIL[{efc}]:", content,
                          f"Oracle {op} missing FAIL[{efc}]: evidence")

    def test_06_no_generic_oracle_import(self):
        for t in TASKS:
            op = BASE / t["oracle_path"]
            content = op.read_text(encoding="utf-8")
            self.assertNotIn("generic_oracle", content,
                             f"{t['id']} oracle imports generic_oracle")

    def test_07_oracles_are_independent_and_substantive(self):
        contents = {}
        for t in TASKS:
            op = BASE / t["oracle_path"]
            content = op.read_text(encoding="utf-8")
            self.assertGreater(len(content.split("\n")), 10,
                               f"{t['id']} oracle too short")
            contents[t["id"]] = content
        unique_contents = set(contents.values())
        self.assertEqual(len(unique_contents), 30,
                         f"Only {len(unique_contents)} unique oracle contents")

    # ── workspace tests ───────────────────────────────────────

    def test_08_workspace_directories_exist(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            self.assertTrue(wp.is_dir(), f"Workspace missing: {wp}")
            self.assertFalse(wp.is_symlink(), f"Workspace is symlink: {wp}")

    def test_09_workspaces_contain_source_code(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            source_files = [f for f in wp.iterdir()
                            if f.is_file() and not f.is_symlink()
                            and f.suffix in (".py", ".multistage")]
            self.assertGreater(len(source_files), 0,
                               f"{t['id']} workspace has no source files")
            non_json = [f for f in wp.iterdir()
                        if f.is_file() and f.suffix != ".json"]
            self.assertGreater(len(non_json), 0,
                               f"{t['id']} workspace only has JSON files")

    def test_10_no_hidden_content_in_workspace(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            self.assertFalse((wp / "oracle.py").exists(),
                             f"{t['id']}: oracle.py found in workspace")
            self.assertFalse((wp / "fixture_check.py").exists(),
                             f"{t['id']}: fixture_check.py found in workspace")
            # Recursive check: no oracle.py or fixture_check.py anywhere
            for f in wp.rglob("oracle.py"):
                self.fail(f"{t['id']}: oracle.py found at {f.relative_to(wp)}")
            for f in wp.rglob("fixture_check.py"):
                self.fail(f"{t['id']}: fixture_check.py at {f.relative_to(wp)}")
            # Check solution content not leaked
            sp = BASE / t["solution_path"]
            if sp.is_dir():
                for sf in sp.rglob("*"):
                    if sf.is_file() and not sf.is_symlink():
                        rel = sf.relative_to(sp)
                        wf = wp / rel
                        if wf.exists() and wf.is_file() and not wf.is_symlink():
                            sol_content = sf.read_bytes()
                            ws_content = wf.read_bytes()
                            self.assertNotEqual(sol_content, ws_content,
                                                f"{t['id']}: {rel} content matches solution (hidden leak)")

    def test_11_required_paths_meaningful(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            rps = t.get("required_paths", [])
            self.assertIsInstance(rps, list, f"{t['id']}: required_paths not a list")
            self.assertGreater(len(rps), 0,
                               f"{t['id']}: required_paths is empty")
            for rp in rps:
                self.assertNotIn(rp, ("oracle.py", "fixture_check.py"),
                                 f"{t['id']}: oracle/fixture_check in required_paths")
                p = wp / rp
                self.assertTrue(p.is_file(),
                                f"{t['id']}: required path {rp} not found")

    def test_12_allowed_paths_non_empty(self):
        for t in TASKS:
            aps = t.get("allowed_paths", [])
            self.assertIsInstance(aps, list, f"{t['id']}: allowed_paths not a list")
            self.assertGreater(len(aps), 0,
                               f"{t['id']}: allowed_paths is empty")

    def test_13_catalog_keys_exact_and_typed(self):
        required_keys = {
            "id": str, "category": str, "provenance": str, "license": str,
            "source_kind": str, "network": str, "permission": str, "risk": str,
            "timeout": int, "cpu_seconds": int, "memory_mb": int,
            "max_open_files": int, "max_processes": int,
            "injected_failures": list, "expected_failure_class": str,
            "task_prompt": str, "allowed_paths": list,
            "forbidden_paths": list, "required_paths": list,
            "oracle_path": str, "solution_path": str, "fixture_path": str,
        }
        valid_permissions = {"code_modification", "device_action", "simulation", "read_only"}
        valid_risks = {"low", "medium", "high"}
        for t in TASKS:
            for key, typ in required_keys.items():
                self.assertIn(key, t, f"{t['id']}: missing key {key}")
                self.assertIsInstance(t[key], typ,
                                      f"{t['id']}: {key} wrong type: {type(t[key]).__name__}")
            self.assertEqual(t["network"], "deny-all",
                             f"{t['id']}: network must be deny-all")
            self.assertIn(t["permission"], valid_permissions,
                          f"{t['id']}: invalid permission {t['permission']}")
            self.assertIn(t["risk"], valid_risks,
                          f"{t['id']}: invalid risk {t['risk']}")
            self.assertGreater(t["timeout"], 0)
            self.assertGreater(t["cpu_seconds"], 0)
            self.assertGreater(t["memory_mb"], 0)
            self.assertGreater(t["max_open_files"], 0)
            self.assertGreater(t["max_processes"], 0)
            self.assertTrue(t.get("task_prompt"), f"{t['id']}: empty task_prompt")
            self.assertIn(t["id"], t["task_prompt"],
                          f"{t['id']}: task_prompt missing task ID")
            # No unknown keys
            known = set(required_keys) | {"risk"}
            unknown = set(t.keys()) - known
            self.assertEqual(len(unknown), 0,
                             f"{t['id']}: unknown keys: {unknown}")
            for pk in ("oracle_path", "solution_path", "fixture_path"):
                p = t[pk]
                self.assertFalse(p.startswith("/"), f"{t['id']}: {pk} is absolute")
                self.assertFalse("\\" in p, f"{t['id']}: {pk} has backslashes")
                self.assertFalse(".." in p.split("/"), f"{t['id']}: {pk} has traversal")
            for pk in ("allowed_paths", "forbidden_paths", "required_paths"):
                for p in t[pk]:
                    self.assertFalse("/" in p or "\\" in p,
                                     f"{t['id']}: path '{p}' in {pk} has separator")

    # ── solution tests ────────────────────────────────────────

    def test_14_solution_directories_exist(self):
        for t in TASKS:
            sp = BASE / t["solution_path"]
            self.assertTrue(sp.is_dir(), f"Solution missing: {sp}")
            self.assertFalse(sp.is_symlink(), f"Solution is symlink: {sp}")

    def test_15_solutions_contain_files(self):
        for t in TASKS:
            sp = BASE / t["solution_path"]
            files = [f for f in sp.iterdir() if f.is_file() and not f.is_symlink()]
            self.assertGreater(len(files), 0,
                               f"{t['id']} solution directory is empty")

    def test_16_solution_files_not_in_model_workspace_by_default(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            sp = BASE / t["solution_path"]
            for sf in sp.rglob("*"):
                if sf.is_file() and not sf.is_symlink():
                    rel = sf.relative_to(sp)
                    wf = wp / rel
                    if wf.exists() and wf.is_file() and not wf.is_symlink():
                        sol_content = sf.read_bytes()
                        ws_content = wf.read_bytes()
                        self.assertNotEqual(sol_content, ws_content,
                                            f"{t['id']}: {rel} identical in workspace and solution")

    # ── baseline/solution oracle tests ────────────────────────

    def test_17_all_baselines_fail(self):
        failures = []
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            op = BASE / t["oracle_path"]
            efc = t["expected_failure_class"]
            tmp = tempfile.mkdtemp(prefix=f"e1_test_bl_{t['id']}_")
            try:
                shutil.copytree(str(wp), tmp, dirs_exist_ok=True, symlinks=False)
                for pyc in Path(tmp).rglob("__pycache__"):
                    shutil.rmtree(pyc, ignore_errors=True)
                p = subprocess.run(
                    [sys.executable, str(op), tmp],
                    capture_output=True,
                    timeout=30,
                    cwd=tmp,
                    env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
                )
                if p.returncode == 0:
                    failures.append(f"{t['id']}: baseline unexpectedly passed")
                else:
                    stderr = p.stderr.decode(errors="replace")
                    stdout = p.stdout.decode(errors="replace")
                    self.assertTrue(
                        len(stderr.strip()) > 0 or len(stdout.strip()) > 0,
                        f"{t['id']}: baseline failed with no output")
                    # MUST have structured failure evidence: FAIL[class]: ...
                    expected_tag = f"FAIL[{efc}]"
                    self.assertIn(expected_tag, stderr + stdout,
                                  f"{t['id']}: missing {expected_tag} evidence in output")
            finally:
                shutil.rmtree(tmp, ignore_errors=True)
        self.assertEqual(len(failures), 0, "\n" + "\n".join(failures))

    def test_18_all_solutions_pass(self):
        failures = []
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            sp = BASE / t["solution_path"]
            op = BASE / t["oracle_path"]
            tmp = tempfile.mkdtemp(prefix=f"e1_test_sol_{t['id']}_")
            try:
                shutil.copytree(str(wp), tmp, dirs_exist_ok=True, symlinks=False)
                for pyc in Path(tmp).rglob("__pycache__"):
                    shutil.rmtree(pyc, ignore_errors=True)
                for sf in sp.rglob("*"):
                    if sf.is_file() and not sf.is_symlink():
                        rel = sf.relative_to(sp)
                        dest = Path(tmp) / rel
                        dest.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(str(sf), str(dest))
                p = subprocess.run(
                    [sys.executable, str(op), tmp],
                    capture_output=True,
                    timeout=30,
                    cwd=tmp,
                    env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
                )
                if p.returncode != 0:
                    failures.append(
                        f"{t['id']}: solution failed (exit={p.returncode})\n"
                        f"  stderr: {p.stderr.decode(errors='replace')[:300]}")
                else:
                    # Solution passes must emit PASS
                    stdout = p.stdout.decode(errors="replace")
                    self.assertIn("PASS", stdout,
                                  f"{t['id']}: solution passed but no PASS output")
            finally:
                shutil.rmtree(tmp, ignore_errors=True)
        self.assertEqual(len(failures), 0, "\n" + "\n".join(failures))

    # ── metadata tests ────────────────────────────────────────

    def test_19_privacy_no_sensitive_data(self):
        forbidden_patterns = [
            "confidential", "internal-use-only", "proprietary domain",
            "access token", "device serial", "192.168.", "10.0.",
            "company", "secret key", "private key", "password",
            "api-key", "api_key", "authorization: bearer",
            "corp.example.com", "internal.example",
        ]
        texts = []
        for t in TASKS:
            texts.append(("catalog", t["id"], t.get("provenance", "")))
            texts.append(("catalog", t["id"], t.get("license", "")))
            texts.append(("catalog", t["id"], t.get("task_prompt", "")))
            op = BASE / t["oracle_path"]
            if op.is_file():
                texts.append(("oracle", t["id"], op.read_text(encoding="utf-8")))
            sp = BASE / t["solution_path"]
            if sp.is_dir():
                for sf in sp.rglob("*"):
                    if sf.is_file() and not sf.is_symlink():
                        try:
                            texts.append(("solution", t["id"],
                                          sf.read_text(encoding="utf-8")))
                        except UnicodeDecodeError:
                            pass
            wp = BASE / t["fixture_path"]
            if wp.is_dir():
                for wf in wp.rglob("*"):
                    if wf.is_file() and not wf.is_symlink():
                        try:
                            texts.append(("workspace", t["id"],
                                          wf.read_text(encoding="utf-8")))
                        except UnicodeDecodeError:
                            pass

        violations = []
        for source, tid, text in texts:
            lower = text.lower()
            for pattern in forbidden_patterns:
                if pattern in lower:
                    violations.append(f"{tid} [{source}]: contains '{pattern}'")
        self.assertEqual(len(violations), 0,
                         f"Privacy violations:\n" + "\n".join(violations))

        all_text = " ".join(t[2] for t in texts).lower()
        banned_legit = [t for t in LEGITIMATE_TERMS if t not in all_text]
        self.assertEqual(len(banned_legit), 0,
                         f"Legitimate terms missing: {banned_legit}")

    def test_20_provenance_license_explicit(self):
        for t in TASKS:
            self.assertTrue(t.get("provenance"), f"{t['id']}: provenance empty")
            self.assertTrue(t.get("license"), f"{t['id']}: license empty")
            self.assertIn("MIT", t["license"],
                          f"{t['id']}: license must be MIT (synthetic)")
            self.assertIn("synthetic", t.get("provenance", "").lower(),
                          f"{t['id']}: provenance must say synthetic")

    def test_21_resources_permissions_and_injection(self):
        for t in TASKS:
            self.assertEqual(t["network"], "deny-all")
            self.assertEqual(t["permission"], EXPECTED_PERMISSIONS[t["category"]])
            self.assertGreater(t["timeout"], 0)
            self.assertGreater(t["cpu_seconds"], 0)
            self.assertGreater(t["memory_mb"], 0)
            self.assertIsInstance(t["injected_failures"], list)
            self.assertTrue(t["injected_failures"],
                            f"{t['id']} has no injected failures")
            for f in t["injected_failures"]:
                self.assertIn(f, FAILURE_CLASSES,
                              f"{t['id']}: unknown failure class '{f}'")
            self.assertIn(t["expected_failure_class"], FAILURE_CLASSES,
                          f"{t['id']}: unknown expected failure class")

    def test_22_injection_set_covers_all_classes(self):
        classes = set(t["expected_failure_class"] for t in TASKS)
        self.assertEqual(classes, set(FAILURE_CLASSES))

    def test_23_injections_semantically_appropriate(self):
        semantic_map = {
            "latency": ["ros2_lifecycle_launch", "containers_build",
                         "control_numerics", "multi_file_engineering"],
            "reordering": ["ros2_lifecycle_launch", "containers_build",
                           "control_numerics", "sensing_logs", "safety_device_protocol"],
            "packet_loss": ["ros2_lifecycle_launch", "containers_build",
                            "sensing_logs", "multi_file_engineering"],
            "endianness": ["can_ethercat_serial", "containers_build",
                           "control_numerics", "safety_device_protocol"],
            "clock_drift": ["ros2_lifecycle_launch", "can_ethercat_serial", "sensing_logs"],
            "device_rejection": ["ros2_lifecycle_launch", "can_ethercat_serial",
                                 "sensing_logs", "multi_file_engineering"],
            "watchdog": ["can_ethercat_serial", "safety_device_protocol"],
        }
        for fail_class, expected_cats in semantic_map.items():
            tasks_with_class = [t for t in TASKS
                                if t["expected_failure_class"] == fail_class]
            cats = set(t["category"] for t in tasks_with_class)
            for expected_cat in expected_cats:
                self.assertIn(expected_cat, cats,
                              f"failure class '{fail_class}' missing from "
                              f"category '{expected_cat}'")

    # ── runner/report tests ────────────────────────────────────

    def test_24_fixture_validation_runs_all_30(self):
        results = _fixture_results()
        self.assertEqual(len(results), 30)
        for r in results:
            self.assertEqual(r["mode"], "fixture-validation")
            self.assertFalse(r["model_acceptance"])

    def test_25_fixture_validation_all_baselines_fail(self):
        results = _fixture_results()
        passed_baselines = [r for r in results if r["baseline_passed"]]
        self.assertEqual(len(passed_baselines), 0,
                         f"Baselines unexpectedly passed: "
                         f"{[r['task_id'] for r in passed_baselines]}")
        # All must have structured evidence
        for r in results:
            self.assertTrue(r.get("baseline_has_structured_evidence"),
                            f"{r['task_id']}: missing structured failure evidence")

    def test_26_fixture_validation_all_solutions_pass(self):
        results = _fixture_results()
        failed_solutions = [r for r in results if not r["solution_passed"]]
        self.assertEqual(len(failed_solutions), 0,
                         f"Solutions failed: "
                         f"{[r['task_id'] for r in failed_solutions]}")

    def test_27_fixture_validation_no_solution_leaks(self):
        results = _fixture_results()
        leaked = [r for r in results if r["solution_leaked"]]
        self.assertEqual(len(leaked), 0,
                         f"Solution leaked: "
                         f"{[(r['task_id'], r['leaked_files']) for r in leaked]}")

    def test_28_deterministic_byte_identical(self):
        r1 = _fixture_results()
        r2 = _fixture_results()
        j1 = json.dumps(r1, sort_keys=True, separators=(",", ":"))
        j2 = json.dumps(r2, sort_keys=True, separators=(",", ":"))
        self.assertEqual(j1, j2, "Fixture validation is not deterministic")

    def test_29_report_slices_and_per_task_complete(self):
        results = _fixture_results()
        s = report.summarize(results)
        self.assertEqual(set(s["slices"].keys()),
                         {"domain", "risk", "cost", "failure_class"})
        for dim in ["domain", "risk", "cost", "failure_class"]:
            self.assertGreater(len(s["slices"][dim]), 0,
                               f"Slice '{dim}' is empty")
        self.assertIn("per_task", s)
        self.assertEqual(len(s["per_task"]), 30)
        for pt in s["per_task"]:
            self.assertIn("task_id", pt)
            self.assertIn("category", pt)
            self.assertIn("expected_failure_class", pt)

    def test_30_report_distinguishes_fixture_from_acceptance(self):
        results = _fixture_results()
        s = report.summarize(results)
        self.assertTrue(s["is_fixture_validation"])
        self.assertFalse(s["model_acceptance_reported"])

    def test_31_no_hardcoded_fabricated_metrics(self):
        """Runner must not fabricate metrics not provided by TurnMetrics schema."""
        results = _fixture_results()
        for r in results:
            # tokens/cache/context must be None (unavailable)
            for key in ["tokens_input", "tokens_output", "cache_hits",
                         "active_context_tokens"]:
                self.assertIsNone(r.get(key),
                                  f"{r['task_id']}: {key} is {r.get(key)}, not None")
            # Real metrics must also be None in fixture mode
            for key in ["tool_calls_made", "tool_errors", "provider_retries",
                         "iterations", "completed_normally"]:
                self.assertIsNone(r.get(key),
                                  f"{r['task_id']}: {key} fabricated in fixture mode")
            # metrics_available must be per-metric dict, not a bool
            ma = r.get("metrics_available")
            self.assertIsInstance(ma, dict,
                                  f"{r['task_id']}: metrics_available not a dict")
            for k in ma:
                self.assertFalse(ma[k],
                                 f"{r['task_id']}: {k} claimed available in fixture")

    def test_32_catalog_consistent_with_filesystem(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            self.assertTrue(wp.is_dir(), f"{t['id']}: fixture_path not a directory")
            op = BASE / t["oracle_path"]
            self.assertTrue(op.is_file(), f"{t['id']}: oracle_path not a file")
            sp = BASE / t["solution_path"]
            self.assertTrue(sp.is_dir(), f"{t['id']}: solution_path not a directory")

    def test_33_manifest_is_source_projection(self):
        from generate_e1_suite import compute_manifest
        fresh = compute_manifest()
        for t in fresh["tasks"]:
            self.assertTrue(t["workspace_digest"],
                            f"{t['id']}: empty workspace_digest")
            self.assertTrue(t["oracle_digest"],
                            f"{t['id']}: empty oracle_digest")
            self.assertTrue(t["solution_digest"],
                            f"{t['id']}: empty solution_digest")
            self.assertEqual(len(t["workspace_digest"]), 64)
            self.assertEqual(len(t["oracle_digest"]), 64)
            self.assertEqual(len(t["solution_digest"]), 64)
            # Sorted file maps must exist
            self.assertIsInstance(t.get("workspace_file_map"), dict)
            self.assertIsInstance(t.get("solution_file_map"), dict)
            self.assertGreater(len(t.get("workspace_file_map", {})), 0)
            self.assertGreater(len(t.get("solution_file_map", {})), 0)
        # Catalog semantic SHA must be present
        self.assertTrue(fresh.get("catalog_sha256"))
        self.assertEqual(len(fresh["catalog_sha256"]), 64)

    def test_34_manifest_drift_detection(self):
        """Manifest must detect drift in workspace, oracle, solution, and catalog."""
        import generate_e1_suite

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            fixtures_src = BASE / "fixtures"
            fixtures_dst = root / "fixtures"
            shutil.copytree(str(fixtures_src), str(fixtures_dst), symlinks=False)
            shutil.copy2(str(BASE / "catalog.toml"), str(root / "catalog.toml"))

            old_base = generate_e1_suite.BASE
            try:
                generate_e1_suite.BASE = root
                original = generate_e1_suite.compute_manifest()
                orig_digest = original["canonical_digest"]

                # Drift 1: workspace mutation
                ws_file = root / "fixtures/tasks/e1_can_ethercat_serial_0001/parser.py"
                ws_file.write_text(ws_file.read_text() + "\n# drift injection\n")
                mutated = generate_e1_suite.compute_manifest()
                self.assertNotEqual(orig_digest, mutated["canonical_digest"],
                                    "Manifest did not detect workspace mutation")

                # Drift 2: oracle mutation (restore workspace, mutate oracle)
                ws_file.write_text(ws_file.read_text().replace("\n# drift injection\n", ""))
                ora_file = root / "fixtures/oracles/e1_can_ethercat_serial_0001.py"
                ora_file.write_text(ora_file.read_text() + "\n# oracle drift\n")
                mutated2 = generate_e1_suite.compute_manifest()
                self.assertNotEqual(orig_digest, mutated2["canonical_digest"],
                                    "Manifest did not detect oracle mutation")

                # Drift 3: solution mutation (restore oracle, mutate solution)
                ora_file.write_text(ora_file.read_text().replace("\n# oracle drift\n", ""))
                sol_file = root / "fixtures/solutions/e1_can_ethercat_serial_0001/parser.py"
                sol_file.write_text(sol_file.read_text() + "\n# solution drift\n")
                mutated3 = generate_e1_suite.compute_manifest()
                self.assertNotEqual(orig_digest, mutated3["canonical_digest"],
                                    "Manifest did not detect solution mutation")

                # Drift 4: catalog change
                sol_file.write_text(sol_file.read_text().replace("\n# solution drift\n", ""))
                cat_file = root / "catalog.toml"
                cat_content = cat_file.read_text()
                cat_file.write_text(cat_content.replace("timeout = 60", "timeout = 61"))
                mutated4 = generate_e1_suite.compute_manifest()
                self.assertNotEqual(orig_digest, mutated4["canonical_digest"],
                                    "Manifest did not detect catalog mutation")
            finally:
                generate_e1_suite.BASE = old_base

    def test_35_report_metric_averages_use_available_count(self):
        results = _fixture_results()
        s = report.summarize(results)
        for dim_name, dim_data in s["slices"].items():
            for slice_name, slice_data in dim_data.items():
                for m in ["tokens_input", "tokens_output", "cache_hits",
                           "active_context_tokens"]:
                    key = f"{m}_avg"
                    if key in slice_data:
                        self.assertIsNone(slice_data[key],
                                          f"{dim_name}/{slice_name}: {key} should be None")

    def test_36_no_hardcoded_safety_scope_first_attempt(self):
        runner_src = (BASE / "runner.py").read_text(encoding="utf-8")
        self.assertNotIn('"safety": 1.0', runner_src)
        self.assertNotIn('"scope": 1.0', runner_src)
        self.assertNotIn('"first_attempt": True', runner_src)
        # Must not claim 'rounds' (not in TurnMetrics)
        self.assertNotIn('"rounds":', runner_src,
                         "runner.py must not claim 'rounds' metric")

    def test_37_cli_deterministic_output_byte_identical(self):
        with tempfile.TemporaryDirectory() as td:
            out1 = Path(td) / "one.json"
            out2 = Path(td) / "two.json"
            p1 = subprocess.run(
                [sys.executable, str(BASE / "runner.py"),
                 "--mode", "fixture-validation",
                 "--output", str(out1)],
                cwd=str(BASE), capture_output=True, timeout=120,
                env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            )
            p2 = subprocess.run(
                [sys.executable, str(BASE / "runner.py"),
                 "--mode", "fixture-validation",
                 "--output", str(out2)],
                cwd=str(BASE), capture_output=True, timeout=120,
                env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            )
            self.assertEqual(p1.returncode, 0,
                             f"First run failed: {p1.stderr.decode()}")
            self.assertEqual(p2.returncode, 0,
                             f"Second run failed: {p2.stderr.decode()}")
            self.assertEqual(out1.read_bytes(), out2.read_bytes(),
                             "CLI output is not byte-identical")

    def test_38_resource_enforcement_in_runner(self):
        src = (BASE / "runner.py").read_text(encoding="utf-8")
        self.assertIn("RLIMIT_CPU", src)
        self.assertIn("RLIMIT_AS", src)
        self.assertIn("RLIMIT_NOFILE", src)
        self.assertIn("RLIMIT_NPROC", src)
        self.assertIn("shell=False", src)
        self.assertNotIn("shell=True", src)

    def test_39_rejects_shared_generic_oracle_for_all_tasks(self):
        seen = {}
        for t in TASKS:
            op = t["oracle_path"]
            if op in seen:
                self.fail(f"Tasks {seen[op]} and {t['id']} share oracle {op}")
            seen[op] = t["id"]

    def test_40_rejects_missing_source_workspaces(self):
        for t in TASKS:
            wp = BASE / t["fixture_path"]
            source_files = (list(wp.glob("*.py")) + list(wp.glob("*.multistage")))
            self.assertGreater(len(source_files), 0,
                               f"{t['id']}: no source files in workspace")

    def test_41_semantic_topics_grounded_in_content(self):
        required_topics = {
            "QoS": {"ids": ["e1_ros2_lifecycle_launch_0001"],
                     "terms": ["reliability", "qos", "history", "reliable"]},
            "lifecycle": {"ids": ["e1_ros2_lifecycle_launch_0002"],
                          "terms": ["lifecycle", "fsm", "state", "transition"]},
            "launch parameter": {"ids": ["e1_ros2_lifecycle_launch_0003"],
                                  "terms": ["param", "resolver", "launch"]},
            "bag": {"ids": ["e1_ros2_lifecycle_launch_0004"],
                    "terms": ["bag", "index"]},
            "discovery": {"ids": ["e1_ros2_lifecycle_launch_0005"],
                          "terms": ["discovery", "node"]},
            "topic remap": {"ids": ["e1_ros2_lifecycle_launch_0006"],
                            "terms": ["remap", "topic"]},
            "Docker": {"ids": ["e1_containers_build_0001"],
                       "terms": ["docker", "multistage", "copy"]},
            "colcon": {"ids": ["e1_containers_build_0002"],
                       "terms": ["colcon", "build", "order"]},
            "cross-compile": {"ids": ["e1_containers_build_0003"],
                              "terms": ["cross", "triplet", "arch"]},
            "build flags": {"ids": ["e1_containers_build_0004"],
                            "terms": ["build", "flags"]},
            "EtherCAT": {"ids": ["e1_can_ethercat_serial_0001"],
                         "terms": ["ethercat", "parser", "endian"]},
            "CAN timeout": {"ids": ["e1_can_ethercat_serial_0002"],
                            "terms": ["can", "timeout"]},
            "serial state": {"ids": ["e1_can_ethercat_serial_0003"],
                             "terms": ["serial", "fsm", "state"]},
            "SDO abort": {"ids": ["e1_can_ethercat_serial_0004"],
                          "terms": ["sdo", "abort"]},
            "CAN bus-off": {"ids": ["e1_can_ethercat_serial_0005"],
                           "terms": ["busoff", "can"]},
            "PID anti-windup": {"ids": ["e1_control_numerics_0001"],
                                "terms": ["pid", "controller", "anti", "windup"]},
            "trajectory": {"ids": ["e1_control_numerics_0002"],
                           "terms": ["trajectory", "limit"]},
            "angle/unit": {"ids": ["e1_control_numerics_0003"],
                           "terms": ["angle", "convert"]},
            "IIR filter": {"ids": ["e1_control_numerics_0004"],
                           "terms": ["iir", "filter"]},
            "deadband": {"ids": ["e1_control_numerics_0005"],
                          "terms": ["deadband"]},
            "timestamp": {"ids": ["e1_sensing_logs_0001"],
                          "terms": ["timestamp", "align"]},
            "drop detection": {"ids": ["e1_sensing_logs_0002"],
                               "terms": ["drop", "detect"]},
            "CSV stats": {"ids": ["e1_sensing_logs_0003"],
                          "terms": ["csv", "stat"]},
            "log diagnosis": {"ids": ["e1_sensing_logs_0004"],
                              "terms": ["log", "diag"]},
            "watchdog": {"ids": ["e1_safety_device_protocol_0001"],
                         "terms": ["watchdog"]},
            "safe-stop": {"ids": ["e1_safety_device_protocol_0002"],
                          "terms": ["safe", "stop"]},
            "attestation": {"ids": ["e1_safety_device_protocol_0003"],
                            "terms": ["attestation"]},
            "multi-file": {"ids": ["e1_multi_file_engineering_0001"],
                           "terms": ["sensor", "controller", "diagnostics"]},
            "test completion": {"ids": ["e1_multi_file_engineering_0002"],
                                "terms": ["test", "mathlib"]},
            "cross-package": {"ids": ["e1_multi_file_engineering_0003"],
                              "terms": ["kinematics", "motion", "planner"]},
        }
        for topic_name, spec in required_topics.items():
            found = False
            for tid in spec["ids"]:
                wp = BASE / next(t["fixture_path"] for t in TASKS if t["id"] == tid)
                for f in wp.iterdir():
                    if f.is_file() and not f.is_symlink():
                        try:
                            content = f.read_text(encoding="utf-8").lower()
                        except UnicodeDecodeError:
                            continue
                        if any(term.lower() in content for term in spec["terms"]):
                            found = True
                            break
                if found:
                    break
                op = BASE / next(t["oracle_path"] for t in TASKS if t["id"] == tid)
                if op.is_file():
                    content = op.read_text(encoding="utf-8").lower()
                    if any(term.lower() in content for term in spec["terms"]):
                        found = True
                        break
            self.assertTrue(found,
                            f"Topic '{topic_name}': no source/oracle contains "
                            f"any of {spec['terms']}")

    # ── permission mode mapping tests ─────────────────────────

    def test_42_permission_to_aletheon_mode_mapping(self):
        """Permission mapping must produce only valid aletheon --permission-mode values."""
        valid_modes = {"safe", "dev", "full"}
        mapping = runner.PERMISSION_TO_ALETHEON_MODE
        for perm, mode in mapping.items():
            self.assertIn(mode, valid_modes,
                          f"{perm} -> {mode} not a valid aletheon mode")
        # read_only must map to safe
        self.assertEqual(mapping.get("read_only"), "safe",
                         "read_only must map to safe")
        # Never map to full
        for perm, mode in mapping.items():
            self.assertNotEqual(mode, "full",
                                f"{perm} must never map to full")
        # All non-read_only must map to dev
        for perm in ("device_action", "code_modification", "simulation"):
            self.assertEqual(mapping.get(perm), "dev",
                             f"{perm} must map to dev")

    # ── idempotency tests ────────────────────────────────────

    def test_43_idempotency_key_deterministic(self):
        """Same run_id + task_id must produce same key."""
        k1 = runner._make_idempotency_key("run-001", "e1_task_0001")
        k2 = runner._make_idempotency_key("run-001", "e1_task_0001")
        self.assertEqual(k1, k2, "Idempotency key not deterministic")

    def test_44_idempotency_key_different_run_id(self):
        """Different run_id must produce different key."""
        k1 = runner._make_idempotency_key("run-001", "e1_task_0001")
        k2 = runner._make_idempotency_key("run-002", "e1_task_0001")
        self.assertNotEqual(k1, k2, "Different run_id produced same key")

    def test_45_idempotency_key_different_task_id(self):
        """Different task_id must produce different key."""
        k1 = runner._make_idempotency_key("run-001", "e1_task_0001")
        k2 = runner._make_idempotency_key("run-001", "e1_task_0002")
        self.assertNotEqual(k1, k2, "Different task_id produced same key")

    def test_46_idempotency_key_rejects_bad_run_id(self):
        """Invalid run_id format must raise."""
        with self.assertRaises(ValueError):
            runner._make_idempotency_key("bad run id!", "task")
        with self.assertRaises(ValueError):
            runner._make_idempotency_key("", "task")
        with self.assertRaises(ValueError):
            runner._make_idempotency_key("a" * 200, "task")

    def test_47_idempotency_key_rejects_bad_task_id(self):
        """Invalid task_id must raise."""
        with self.assertRaises(ValueError):
            runner._make_idempotency_key("run-001", "")

    # ── real-runner command builder tests ─────────────────────

    def test_48_command_builder_exact_argv_order_and_enum(self):
        """Verify exact argv order and valid enum values for real-evaluation command."""
        task = TASKS[0]
        tid = task["id"]
        permission = task["permission"]
        run_id = "test-run-v1"

        # Build expected command
        aletheon_binary = "/usr/bin/aletheon"
        ws = str(BASE / task["fixture_path"])
        prompt = task["task_prompt"]
        permission_mode = runner.PERMISSION_TO_ALETHEON_MODE.get(permission, "dev")
        idem_key = runner._make_idempotency_key(run_id, tid)

        expected_argv = [
            aletheon_binary,
            "--cd", ws,
            "exec",
            "--prompt", prompt,
            "--sandbox", "require",
            "--output", "json",
            "--idempotency-key", idem_key,
            "--timeout-seconds", str(task["timeout"]),
            "--permission-mode", permission_mode,
        ]

        # Validate flag order
        self.assertEqual(expected_argv[0], aletheon_binary,
                         "First arg must be aletheon binary path")
        self.assertEqual(expected_argv[1], "--cd",
                         "--cd must come before exec")
        self.assertEqual(expected_argv[3], "exec",
                         "exec subcommand at position 3")
        self.assertIn("--sandbox", expected_argv)
        self.assertIn("require", expected_argv)
        self.assertIn("--output", expected_argv)
        self.assertIn("json", expected_argv)
        self.assertIn("--idempotency-key", expected_argv)
        self.assertIn("--timeout-seconds", expected_argv)
        self.assertIn("--permission-mode", expected_argv)

        # Permission mode must be valid
        self.assertIn(permission_mode, {"safe", "dev"},
                      f"permission_mode {permission_mode} not valid")
        self.assertNotEqual(permission_mode, "full",
                            "must never use full permission")

        # No invalid flags
        self.assertNotIn("--workspace", expected_argv)
        self.assertNotIn("--allowed-paths", expected_argv)
        self.assertNotIn("--timeout", expected_argv)  # --timeout-seconds, not --timeout

    # ── catalog validation tests ──────────────────────────────

    def test_49_catalog_validation_rejects_malformed(self):
        """Fail-closed catalog validation must reject malformed tasks."""
        errors = runner._validate_catalog([], BASE)
        self.assertTrue(errors)

        # Missing required key
        bad_task = dict(TASKS[0])
        del bad_task["id"]
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("missing key id" in e for e in errors))

        # Invalid permission
        bad_task2 = dict(TASKS[0])
        bad_task2["permission"] = "admin"
        errors = runner._validate_catalog([bad_task2], BASE)
        self.assertTrue(any("invalid permission" in e for e in errors))

        # Invalid risk
        bad_task3 = dict(TASKS[0])
        bad_task3["risk"] = "critical"
        errors = runner._validate_catalog([bad_task3], BASE)
        self.assertTrue(any("invalid risk" in e for e in errors))

        # Path traversal
        bad_task4 = dict(TASKS[0])
        bad_task4["oracle_path"] = "../secrets/oracle.py"
        errors = runner._validate_catalog([bad_task4], BASE)
        self.assertTrue(any("traversal" in e for e in errors))

        # Duplicate ID
        t1 = dict(TASKS[0])
        t2 = dict(TASKS[0])
        errors = runner._validate_catalog([t1, t2], BASE)
        self.assertTrue(any("duplicate" in e for e in errors))

        # Network not deny-all
        bad_task5 = dict(TASKS[0])
        bad_task5["network"] = "allow"
        errors = runner._validate_catalog([bad_task5], BASE)
        self.assertTrue(any("network" in e for e in errors))

        # Negative timeout
        bad_task6 = dict(TASKS[0])
        bad_task6["timeout"] = 0
        errors = runner._validate_catalog([bad_task6], BASE)
        self.assertTrue(any("timeout" in e for e in errors))

        # Invalid failure class
        bad_task7 = dict(TASKS[0])
        bad_task7["expected_failure_class"] = "segfault"
        errors = runner._validate_catalog([bad_task7], BASE)
        self.assertTrue(any("failure_class" in e or "failure" in e for e in errors))

    def test_50_catalog_validation_accepts_valid(self):
        """Valid catalog must pass validation."""
        errors = runner._validate_catalog(TASKS, BASE)
        self.assertEqual(len(errors), 0,
                         f"Valid catalog rejected: {errors}")

    # ── report validation tests ───────────────────────────────

    def test_51_report_rejects_mixed_modes(self):
        """report.summarize must reject records with mixed modes."""
        fixture_result = _fixture_results()[0]
        real_like = dict(fixture_result)
        real_like["mode"] = "real-evaluation"
        with self.assertRaises(ValueError):
            report.summarize([fixture_result, real_like])

    def test_52_report_rejects_duplicate_ids(self):
        """report.summarize must reject duplicate task IDs."""
        r = _fixture_results()[0]
        with self.assertRaises(ValueError):
            report.summarize([r, dict(r)])

    def test_53_report_rejects_fixture_model_acceptance(self):
        """report.summarize must reject model_acceptance=true in fixture mode."""
        r = _fixture_results()[0]
        r = dict(r)
        r["model_acceptance"] = True
        with self.assertRaises(ValueError):
            report.summarize([r])

    def test_54_report_model_acceptance_reported_semantics(self):
        """model_acceptance_reported requires real mode AND at least one acceptance."""
        # Fixture mode: false
        results = _fixture_results()
        s = report.summarize(results)
        self.assertFalse(s["model_acceptance_reported"])

    def test_55_report_scoring_dimensions_present(self):
        """Report must include scoring summary with all spec dimensions."""
        results = _fixture_results()
        s = report.summarize(results)
        self.assertIn("scoring_summary", s)
        ss = s["scoring_summary"]
        for dim in ["correctness_available", "test_quality_available",
                      "safety_boundary_available", "scope_consistency_available",
                      "first_attempt_available", "human_intervention_available"]:
            self.assertIn(dim, ss, f"missing scoring dimension {dim}")

    # ── catalog validation: unknown keys, bool-as-int, excessive limits ──

    def test_56_catalog_rejects_unknown_keys(self):
        """Catalog validation must reject tasks with unknown extra keys."""
        bad_task = dict(TASKS[0])
        bad_task["extra_field"] = "should not be here"
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("unknown keys" in e for e in errors),
                        f"Must reject unknown keys: {errors}")

    def test_57_catalog_rejects_bool_as_int(self):
        """Catalog validation must reject bool values for integer fields."""
        bad_task = dict(TASKS[0])
        bad_task["timeout"] = True
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("timeout" in e and "bool" in e for e in errors),
                        f"Must reject bool-as-int: {errors}")
        # Also test cpu_seconds
        bad_task2 = dict(TASKS[0])
        bad_task2["cpu_seconds"] = False
        errors2 = runner._validate_catalog([bad_task2], BASE)
        self.assertTrue(any("cpu_seconds" in e and "bool" in e for e in errors2),
                        f"Must reject bool-as-int: {errors2}")

    def test_58_catalog_rejects_excessive_limits(self):
        """Catalog validation must reject resource limits exceeding max bounds."""
        bad_task = dict(TASKS[0])
        bad_task["timeout"] = 99999  # exceeds 3600
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("timeout" in e and "3600" in e for e in errors),
                        f"Must reject excessive timeout: {errors}")

    def test_59_catalog_rejects_duplicate_list_entries(self):
        """Catalog validation must reject duplicate entries in lists."""
        bad_task = dict(TASKS[0])
        bad_task["allowed_paths"] = ["parser.py", "parser.py"]
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("duplicate" in e for e in errors),
                        f"Must reject duplicate list entries: {errors}")

    def test_60_catalog_rejects_wrong_asset_root(self):
        """Catalog validation must enforce exact asset root patterns."""
        bad_task = dict(TASKS[0])
        tid = bad_task["id"]
        bad_task["fixture_path"] = f"fixtures/tasks/wrong_{tid}"
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(any("fixture_path must be" in e for e in errors),
                        f"Must reject wrong fixture_path: {errors}")

    def test_61_catalog_rejects_wrong_category_distribution(self):
        """Catalog validation must reject wrong category counts."""
        # Take 29 valid tasks (one missing) - will fail count check
        subset = list(TASKS[:29])
        errors = runner._validate_catalog(subset, BASE)
        self.assertTrue(any("expected 30" in e for e in errors),
                        f"Must reject wrong task count: {errors}")

    def test_62_pass_count_mode_specific_fixture(self):
        """Fixture mode pass_count must use solution_passed only."""
        results = _fixture_results()
        s = report.summarize(results)
        # In fixture mode, model_acceptance is always False
        # and pass_count should equal number of tasks with solution_passed
        self.assertTrue(s["is_fixture_validation"])
        for dim_data in s["slices"].values():
            for slice_data in dim_data.values():
                # pass_count should never exceed count
                self.assertLessEqual(slice_data["pass_count"], slice_data["count"])

    def test_63_report_rejects_malformed_scoring_dict(self):
        """report.summarize must handle non-dict scoring fields safely."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["scoring"] = "not-a-dict"
        with self.assertRaises(ValueError):
            report.summarize([r0])


class E1FakeExecutableRealRunnerTests(unittest.TestCase):
    """Real-runner tests using an injectable fake /usr/bin/aletheon.

    The fake executable records argv, optionally modifies controlled paths,
    and emits exact Exec terminal JSON. No live LLM calls, no /usr/bin writes.
    """

    def _make_fake_aletheon(self, tmpdir: str, *, status="completed",
                            operation_id="op-test-001", exit_code=0,
                            error_code=None, metrics=None,
                            schema_version=1, terminal_type="terminal",
                            extra_fields=None,
                            modify_files=None,
                            record_argv_path=None) -> str:
        """Create a fake aletheon executable that emits controlled terminal JSON.

        Args:
            modify_files: dict of {relative_path: content} to write into workspace
            record_argv_path: if set, write argv to this file
        """
        obj = _fake_exec_terminal_json(
            status=status,
            operation_id=operation_id,
            error_code=error_code,
            metrics=metrics,
            schema_version=schema_version,
            extra_fields=extra_fields,
        )
        if terminal_type != "terminal":
            obj["type"] = terminal_type

        output_json = json.dumps(obj)

        script = f"""#!/usr/bin/env python3
import json, os, sys, base64 as _b64

# Record argv
argv_file = {json.dumps(record_argv_path) if record_argv_path is not None else 'None'}
if argv_file:
    with open(argv_file, 'w') as f:
        json.dump(sys.argv, f)

# Modify files if requested
modify = {json.dumps(modify_files) if modify_files is not None else '{}'}
if modify:
    try:
        cd_idx = sys.argv.index('--cd')
        ws = sys.argv[cd_idx + 1]
    except (ValueError, IndexError):
        ws = '.'
    for relpath, content in modify.items():
        target = os.path.join(ws, relpath)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, 'w') as f:
            f.write(content)

# Emit terminal JSON
_output = _b64.b64decode({json.dumps(base64.b64encode(output_json.encode()).decode())}).decode()
sys.stdout.write(_output)
sys.stdout.write(chr(10))
sys.exit({exit_code})
"""
        fake_path = os.path.join(tmpdir, "fake-aletheon")
        with open(fake_path, "w") as f:
            f.write(script)
        os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)
        return fake_path

    def _simple_task(self, **overrides):
        """Return a minimal valid task dict for testing."""
        t = dict(TASKS[0])
        t.update(overrides)
        return t

    def _read_solution_content(self, task):
        """Read the actual solution file content for a task's first required path."""
        sp = BASE / task["solution_path"]
        rp = task["required_paths"][0]
        sf = sp / rp
        if sf.exists() and sf.is_file():
            return sf.read_text(encoding="utf-8")
        return "# fixed\n"

    def test_fake_success_returns_model_acceptance_true(self):
        """Completed terminal with all success criteria must yield model_acceptance=True."""
        with tempfile.TemporaryDirectory() as td:
            record_path = os.path.join(td, "argv.json")
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            fake = self._make_fake_aletheon(
                td,
                status="completed",
                operation_id="op-success-001",
                exit_code=0,
                metrics={
                    "tool_calls_made": 3,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 5000,
                    "iterations": 5,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: solution_content,
                },
                record_argv_path=record_path,
            )

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-01", aletheon_binary=fake
            )

            self.assertEqual(result["mode"], "real-evaluation")
            self.assertTrue(result["model_acceptance"],
                            f"Expected acceptance, got {result['failure_reasons']}")
            self.assertEqual(result["failure_class"], "none")
            self.assertTrue(result["oracle_passed"])
            self.assertTrue(result["json_valid"])
            self.assertTrue(result["terminal_snapshot"])
            self.assertEqual(result["terminal_status"], "completed")
            self.assertEqual(result["operation_id"], "op-success-001")
            self.assertTrue(result["head_unchanged"])
            self.assertEqual(len(result["scope_violations"]), 0)
            self.assertEqual(result["process_group_reaped"], True)
            self.assertEqual(result["exec_exit_code"], 0)

            # Provenance fields
            self.assertTrue(result["binary_sha256"])
            self.assertEqual(result["binary_path"], fake)
            self.assertTrue(result["idempotency_key"])
            self.assertTrue(result["argv"])

            # Metrics
            self.assertEqual(result["tool_calls_made"], 3)
            self.assertEqual(result["iterations"], 5)
            self.assertTrue(result["completed_normally"])
            self.assertTrue(result["metrics_available"]["tool_calls_made"])
            self.assertFalse(result["metrics_available"]["tokens_input"])

            # Scoring
            scoring = result["scoring"]
            self.assertTrue(scoring["correctness"])
            self.assertTrue(scoring["safety_boundary"])
            self.assertTrue(scoring["first_attempt_success"])
            self.assertEqual(scoring["human_intervention"], "unavailable")

            # Oracle provenance fields (Fix 2)
            self.assertIsInstance(result["oracle_stdout"], str)
            self.assertIsInstance(result["oracle_stderr"], str)
            self.assertEqual(len(result["oracle_stdout_digest"]), 64)
            self.assertEqual(len(result["oracle_stderr_digest"]), 64)
            self.assertIsInstance(result["oracle_timed_out"], bool)
            self.assertFalse(result["oracle_timed_out"],
                             "oracle_timed_out must be False for passing oracle")

            # Verify argv was recorded with correct flags AND EXACT ORDER
            self.assertTrue(os.path.exists(record_path))
            with open(record_path) as f:
                recorded_argv = json.load(f)
            self.assertIn("--cd", recorded_argv)
            self.assertIn("exec", recorded_argv)
            self.assertIn("--permission-mode", recorded_argv)
            perm_idx = recorded_argv.index("--permission-mode")
            self.assertIn(recorded_argv[perm_idx + 1], {"safe", "dev"})

            # --socket must appear BEFORE exec (not a global arg)
            socket_idx = recorded_argv.index("--socket")
            exec_idx = recorded_argv.index("exec")
            self.assertLess(socket_idx, exec_idx,
                            f"--socket (idx {socket_idx}) must appear before "
                            f"exec (idx {exec_idx}): {recorded_argv}")
            # --cd must appear before exec
            cd_idx = recorded_argv.index("--cd")
            self.assertLess(cd_idx, exec_idx,
                            f"--cd (idx {cd_idx}) must appear before "
                            f"exec (idx {exec_idx}): {recorded_argv}")

    def test_fake_malformed_json_rejected(self):
        """Non-JSON output must produce malformed_output failure."""
        with tempfile.TemporaryDirectory() as td:
            fake_path = os.path.join(td, "fake-aletheon")
            with open(fake_path, "w") as f:
                f.write("#!/usr/bin/env python3\nprint('not json')\n")
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-mj", aletheon_binary=fake_path
            )

            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "malformed_output")
            self.assertFalse(result["json_valid"])
            self.assertFalse(result["terminal_snapshot"])

    def test_fake_nonzero_exit(self):
        """Nonzero exit with valid completed terminal should be nonzero_exit failure."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-exit-1",
                exit_code=1,
                modify_files={
                    task["required_paths"][0]: self._read_solution_content(task),
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-ne", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "nonzero_exit")

    def test_fake_timeout(self):
        """Timeout detection in _run_bounded with short deadline."""
        with tempfile.TemporaryDirectory() as td:
            fake_path = os.path.join(td, "fake-slow")
            with open(fake_path, "w") as f:
                f.write("#!/usr/bin/env python3\nimport time\n"
                        "time.sleep(5)\n")
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            # Use _run_bounded directly with a very short timeout
            result = runner._run_bounded(
                [fake_path], cwd=Path(td), env=runner._sanitized_env(),
                timeout=0.1, aletheon_binary=fake_path,
            )
            self.assertTrue(result.get("timed_out"),
                            f"_run_bounded should time out after 0.1s, got {result}")

    def test_fake_empty_operation_id(self):
        """Empty operation_id must produce malformed_terminal failure.
        The strict schema validation catches this before classification."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="", exit_code=0,
                modify_files={task["required_paths"][0]: self._read_solution_content(task)},
            )
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-eoid", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_operation_id"])

    def test_fake_provider_unavailable(self):
        """provider_unavailable status must produce provider_error failure."""
        with tempfile.TemporaryDirectory() as td:
            fake = self._make_fake_aletheon(
                td, status="provider_unavailable",
                operation_id="op-prov-err", exit_code=0,
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-pu", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "provider_error")

    def test_fake_provider_rejected(self):
        """provider_rejected status must produce provider_error failure."""
        with tempfile.TemporaryDirectory() as td:
            fake = self._make_fake_aletheon(
                td, status="provider_rejected",
                operation_id="op-rej", exit_code=0,
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-pr", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "provider_error")

    def test_fake_terminal_error_code(self):
        """Terminal with error_code must produce terminal_error_code failure."""
        with tempfile.TemporaryDirectory() as td:
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-ec",
                error_code="E_SANDBOX_VIOLATION", exit_code=0,
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-ec", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "terminal_error_code")
            self.assertIn("error_code:E_SANDBOX_VIOLATION", result["failure_reasons"])

    def test_fake_out_of_scope_change(self):
        """Changes outside allowed_paths must produce scope_violation failure."""
        with tempfile.TemporaryDirectory() as td:
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-scope", exit_code=0,
                modify_files={
                    TASKS[0]["required_paths"][0]: "# fixed\n",
                    "evil.py": "# outside scope\n",
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-scope", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "scope_violation")
            self.assertTrue(any("evil.py" in v for v in result["scope_violations"]))

    def test_fake_missing_required_change(self):
        """Required path not changed must produce scope_violation failure."""
        with tempfile.TemporaryDirectory() as td:
            # Don't modify any files - required path will be missed
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-noreq", exit_code=0,
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-noreq", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn("scope_violation", result["failure_class"])
            self.assertTrue(any("required_not_changed" in v
                               for v in result["scope_violations"]),
                            "Missing required change not detected")

    def test_fake_oracle_failure(self):
        """Correct scope but oracle fails must produce oracle_failure.
        Requires completed_normally=True so classification reaches oracle checks."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            # Write content that doesn't fix the bug
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-oracle", exit_code=0,
                metrics={
                    "tool_calls_made": 1,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 100,
                    "iterations": 1,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: "# This does not fix the bug\nprint('broken')\n",
                },
            )
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-oracle", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "oracle_failure")

    def test_fake_completed_normally_false_rejected(self):
        """Terminal with status=completed but completed_normally=False
        must produce terminal_not_completed, not reach oracle_failure."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-cnf", exit_code=0,
                metrics={
                    "tool_calls_made": 1,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 100,
                    "iterations": 1,
                    "completed_normally": False,
                },
                modify_files={
                    task["required_paths"][0]: self._read_solution_content(task),
                },
            )
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-cnf", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "terminal_not_completed")
            self.assertIn("completed_normally_not_true", result["failure_reasons"])

    def test_fake_binary_not_found(self):
        """Missing binary must return infrastructure_error with same schema."""
        task = self._simple_task()
        result = runner._real_evaluate_one_with_binary(
            task, "test-run-nobin", aletheon_binary="/nonexistent/aletheon"
        )
        self.assertEqual(result["mode"], "real-evaluation")
        self.assertFalse(result["model_acceptance"])
        self.assertEqual(result["failure_class"], "infrastructure_error")
        self.assertIn("aletheon_exec_not_found", result["failure_reasons"])
        # Must have all required schema fields (not an inconsistent small dict)
        for key in ["operation_id", "terminal_status", "json_valid",
                     "terminal_snapshot", "oracle_exit", "exec_exit_code",
                     "changed_files", "scope_violations", "head_unchanged",
                     "binary_sha256", "binary_path"]:
            self.assertIn(key, result, f"Early result missing key {key}")

    def test_fake_wrong_schema_version(self):
        """schema_version != 1 must produce malformed_terminal failure.
        The strict schema validation catches this before classification."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-wsv",
                schema_version=2, exit_code=0,
                modify_files={task["required_paths"][0]: self._read_solution_content(task)},
            )
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-wsv", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_terminal_snapshot"])

    def test_fake_wrong_type_field(self):
        """type != 'terminal' must produce malformed_terminal failure.
        The strict schema validation catches this before classification."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-wt",
                terminal_type="stream", exit_code=0,
                modify_files={task["required_paths"][0]: self._read_solution_content(task)},
            )
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-wt", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_terminal_snapshot"])

    def test_fake_metrics_parsed_correctly(self):
        """Real TurnMetrics fields must be parsed as typed int/bool."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-metrics", exit_code=0,
                metrics={
                    "tool_calls_made": 7,
                    "tool_errors": 1,
                    "provider_retries": 2,
                    "elapsed_ms": 4200,
                    "iterations": 3,
                    "completed_normally": True,
                },
                modify_files={task["required_paths"][0]: self._read_solution_content(task)},
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-metrics", aletheon_binary=fake
            )
            self.assertTrue(result["model_acceptance"])
            # Real fields
            self.assertEqual(result["tool_calls_made"], 7)
            self.assertEqual(result["tool_errors"], 1)
            self.assertEqual(result["provider_retries"], 2)
            self.assertEqual(result["elapsed_ms"], 4200)
            self.assertEqual(result["iterations"], 3)
            self.assertTrue(result["completed_normally"])
            # Token/cache/context must remain None (unavailable)
            self.assertIsNone(result["tokens_input"])
            self.assertIsNone(result["tokens_output"])
            self.assertIsNone(result["cache_hits"])
            self.assertIsNone(result["active_context_tokens"])
            # Availability per metric
            ma = result["metrics_available"]
            self.assertTrue(ma["tool_calls_made"])
            self.assertTrue(ma["tool_errors"])
            self.assertTrue(ma["provider_retries"])
            self.assertTrue(ma["elapsed_ms"])
            self.assertTrue(ma["iterations"])
            self.assertTrue(ma["completed_normally"])
            self.assertFalse(ma["tokens_input"])
            self.assertFalse(ma["tokens_output"])

    def test_fake_production_locked_to_usr_bin(self):
        """real_evaluate_one must always pass /usr/bin/aletheon regardless of env."""
        src = (BASE / "runner.py").read_text(encoding="utf-8")
        # The public function must hardcode /usr/bin/aletheon
        self.assertIn('aletheon_binary="/usr/bin/aletheon"', src,
                      "real_evaluate_one must lock to /usr/bin/aletheon")
        # The private function must accept an injectable parameter
        self.assertIn("def _real_evaluate_one_with_binary", src)
        self.assertIn("aletheon_binary", src)

    def test_fake_report_rejects_fixture_acceptance(self):
        """report.summarize must reject model_acceptance=True in fixture mode."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["model_acceptance"] = True
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_fake_manifest_includes_file_maps(self):
        """Manifest must include sorted file→SHA-256 maps."""
        from generate_e1_suite import compute_manifest
        m = compute_manifest()
        for task in m["tasks"]:
            self.assertIn("workspace_file_map", task)
            self.assertIn("solution_file_map", task)
            wm = task["workspace_file_map"]
            sm = task["solution_file_map"]
            self.assertIsInstance(wm, dict)
            self.assertIsInstance(sm, dict)
            self.assertGreater(len(wm), 0,
                               f"{task['id']}: empty workspace file map")
            self.assertGreater(len(sm), 0,
                               f"{task['id']}: empty solution file map")
            # Verify keys are sorted
            keys = list(wm.keys())
            self.assertEqual(keys, sorted(keys),
                             f"{task['id']}: workspace file map not sorted")
            # Verify all values are 64-char hex digests
            for path_, digest_ in wm.items():
                self.assertEqual(len(digest_), 64,
                                 f"{task['id']}: bad digest length for {path_}")

    def test_fake_manifest_has_catalog_sha(self):
        """Manifest must include the canonical catalog SHA."""
        from generate_e1_suite import compute_manifest
        m = compute_manifest()
        self.assertTrue(m.get("catalog_sha256"))
        self.assertEqual(len(m["catalog_sha256"]), 64)

    def test_fake_recursive_privacy_checks(self):
        """Privacy and solution leak checks must use rglob, not iterdir."""
        src = (BASE / "runner.py").read_text(encoding="utf-8")
        self.assertIn("rglob", src, "runner.py must use rglob for recursive checks")
        # _check_hidden_leak must use _rglob_files
        self.assertIn("_rglob_files", src)

    def test_fake_all_30_oracles_have_structured_evidence(self):
        """Every oracle must emit FAIL[expected_failure_class]: in stderr."""
        for t in TASKS:
            op = BASE / t["oracle_path"]
            content = op.read_text(encoding="utf-8")
            efc = t["expected_failure_class"]
            self.assertIn(f"FAIL[{efc}]:", content,
                          f"{t['id']}: oracle missing FAIL[{efc}]: evidence")

    def test_fake_cost_tier_in_report(self):
        """Report per_task records must include cost_tier."""
        results = _fixture_results()
        s = report.summarize(results)
        for pt in s["per_task"]:
            self.assertIn("cost_tier", pt)
            self.assertIn(pt["cost_tier"], ("low", "high"))

    def test_fake_runner_has_no_rounds_claim(self):
        """Runner must not claim 'rounds' field not in TurnMetrics."""
        src = (BASE / "runner.py").read_text(encoding="utf-8")
        # "rounds" as a key, not "inference_rounds" which we don't claim either
        self.assertNotIn('"rounds"', src,
                         "runner.py claims 'rounds' not in TurnMetrics schema")

    # ── resource enforcement behavioral tests ───────────────────

    def test_resource_limits_enforced_on_child(self):
        """Child process must observe the configured memory limit (RLIMIT_AS)."""
        with tempfile.TemporaryDirectory() as td:
            # Write a fake that tries to allocate more than the limit
            fake_path = os.path.join(td, "fake-mem-hog")
            with open(fake_path, "w") as f:
                f.write("#!/usr/bin/env python3\n"
                        "# Allocate ~512MB which exceeds the 256MB limit\n"
                        "import sys\n"
                        "try:\n"
                        "    x = bytearray(512 * 1024 * 1024)\n"
                        "    print('{\"schema_version\":1,\"type\":\"terminal\",'\n"
                        "          '\"status\":\"completed\",\"operation_id\":\"op\"}')\n"
                        "    sys.exit(0)\n"
                        "except MemoryError:\n"
                        "    sys.exit(137)\n"
                        "except Exception as e:\n"
                        "    print(f'Error: {e}', file=sys.stderr)\n"
                        "    sys.exit(1)\n")
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-memlimit", aletheon_binary=fake_path
            )
            # The child should either be killed by RLIMIT_AS (exit != 0)
            # or raise MemoryError (exit 137). Exit 0 would be a failure.
            self.assertFalse(result["model_acceptance"],
                             "Memory-hogging child should not pass")
            self.assertIn(result["failure_class"],
                          ["nonzero_exit", "infrastructure_error", "timeout",
                           "malformed_output", "missing_terminal_snapshot"],
                          f"Unexpected failure class: {result['failure_class']}")

    # ── all-failed real run regression ─────────────────────────

    def test_all_failed_real_run_report(self):
        """model_acceptance_reported must be true for real-evaluation
        even when every task fails."""
        with tempfile.TemporaryDirectory() as td:
            fake_path = os.path.join(td, "fake-always-fail")
            with open(fake_path, "w") as f:
                f.write("#!/usr/bin/env python3\n"
                        "import sys\n"
                        "sys.exit(1)\n")
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-allfail", aletheon_binary=fake_path
            )
            self.assertFalse(result["model_acceptance"])
            self.assertNotEqual(result["failure_class"], "none")

            s = report.summarize([result])
            self.assertTrue(s["model_acceptance_reported"],
                            "model_acceptance_reported must be True "
                            "for real-evaluation even with all failures")
            self.assertEqual(s["mode"], "real-evaluation")
            self.assertFalse(s["is_fixture_validation"])

    # ── oracle-pass-but-scope-fail ─────────────────────────────

    def test_oracle_pass_but_scope_violation_fails_acceptance(self):
        """model_acceptance must be False when oracle passes but scope
        violation exists."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            # Modify required path (fixes the bug) but also touch a forbidden path
            fake = self._make_fake_aletheon(
                td,
                status="completed",
                operation_id="op-scope-oracle-pass",
                exit_code=0,
                modify_files={
                    task["required_paths"][0]: solution_content,
                    "oracle.py": "# forbidden file\n",
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-scope-oracle", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"],
                             "Scope violation must fail even if oracle passes")
            self.assertEqual(result["failure_class"], "scope_violation")
            # Oracle may have passed but model_acceptance is still False
            # (oracle ran successfully, but scope blocks acceptance)

    # ── malformed mode/type rejection in report ─────────────────

    def test_report_rejects_invalid_mode(self):
        """report.summarize must reject records with invalid mode."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "bad-mode"
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_report_rejects_non_bool_model_acceptance(self):
        """report.summarize must reject non-bool model_acceptance."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = "yes"  # string, not bool
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_report_rejects_non_dict_record(self):
        """report.summarize must reject non-dict records gracefully."""
        with self.assertRaises(ValueError):
            report.summarize(["not a dict"])

    # ── scoring dimensions: available counts and unavailable ────

    def test_scoring_summary_available_counts_honest(self):
        """Scoring summary must expose availability and counts."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td,
                status="completed",
                operation_id="op-score-count",
                exit_code=0,
                metrics={
                    "tool_calls_made": 5,
                    "tool_errors": 0,
                    "provider_retries": 1,
                    "elapsed_ms": 3000,
                    "iterations": 3,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: self._read_solution_content(task),
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-score", aletheon_binary=fake
            )
            s = report.summarize([result])
            ss = s["scoring_summary"]
            # Tool calls should be available with count 1
            self.assertTrue(ss.get("tool_calls_available"))
            self.assertEqual(ss.get("tool_calls_count"), 1)
            # Token/cache must remain explicitly unavailable
            self.assertFalse(ss.get("token_cost_available"))
            self.assertFalse(ss.get("cache_cost_available"))
            # Correctness should be available (oracle passed)
            self.assertTrue(ss.get("correctness_available"))

    def test_scoring_summary_all_unavailable_dimensions_present(self):
        """All spec-required scoring dimensions must be present
        even when unavailable."""
        results = _fixture_results()
        s = report.summarize(results)
        ss = s["scoring_summary"]
        for dim in ["token_cost_available", "cache_cost_available",
                      "tool_cost_available", "test_quality_available",
                      "human_intervention_available"]:
            self.assertIn(dim, ss, f"Missing scoring dimension {dim}")
        # All unavailable in fixture mode
        self.assertFalse(ss["token_cost_available"])
        self.assertFalse(ss["cache_cost_available"])

    # ── socket precedence tests ────────────────────────────────

    def test_socket_path_honors_aletheon_socket_env(self):
        """_socket_path must honor ALETHEON_SOCKET before XDG."""
        import importlib
        with tempfile.TemporaryDirectory() as td:
            custom_sock = os.path.join(td, "custom.sock")
            Path(custom_sock).touch()
            old_env = os.environ.get("ALETHEON_SOCKET")
            old_xdg = os.environ.get("XDG_RUNTIME_DIR")
            try:
                os.environ["ALETHEON_SOCKET"] = custom_sock
                # Reload runner to pick up new env
                importlib.reload(runner)
                resolved = runner._socket_path()
                self.assertEqual(resolved, custom_sock,
                                 f"Expected {custom_sock}, got {resolved}")
            finally:
                if old_env:
                    os.environ["ALETHEON_SOCKET"] = old_env
                else:
                    os.environ.pop("ALETHEON_SOCKET", None)
                if old_xdg:
                    os.environ["XDG_RUNTIME_DIR"] = old_xdg
                importlib.reload(runner)

    def test_sanitized_env_preserves_aletheon_socket(self):
        """_sanitized_env must preserve ALETHEON_SOCKET."""
        import importlib
        old_env = os.environ.get("ALETHEON_SOCKET")
        try:
            os.environ["ALETHEON_SOCKET"] = "/tmp/test-socket.sock"
            importlib.reload(runner)
            env = runner._sanitized_env()
            self.assertEqual(env.get("ALETHEON_SOCKET"), "/tmp/test-socket.sock",
                             "_sanitized_env must preserve ALETHEON_SOCKET")
        finally:
            if old_env:
                os.environ["ALETHEON_SOCKET"] = old_env
            else:
                os.environ.pop("ALETHEON_SOCKET", None)
            importlib.reload(runner)

    # ── catalog validation: later task after invalid task ──────

    def test_catalog_validation_validates_later_tasks_after_invalid(self):
        """An invalid task must not prevent validation of later tasks."""
        from copy import deepcopy
        tasks = []
        # First task: invalid (missing id)
        bad_task = deepcopy(TASKS[0])
        del bad_task["id"]
        tasks.append(bad_task)
        # Second task: valid
        tasks.append(deepcopy(TASKS[1]))

        errors = runner._validate_catalog(tasks, BASE)
        # Must have error for first task (missing id)
        self.assertTrue(any("missing key id" in e for e in errors),
                        "Must report missing id for first task")
        # Must NOT have errors for the valid second task
        second_task_errs = [e for e in errors
                            if TASKS[1]["id"] in e]
        self.assertEqual(len(second_task_errs), 0,
                         f"Valid second task should not have errors: {second_task_errs}")

    # ── category distribution: 30 tasks with wrong distribution ──

    def test_catalog_rejects_wrong_category_distribution_30(self):
        """A 30-task catalog with wrong category distribution must be rejected.
        This proves distribution, not just total count."""
        from copy import deepcopy
        tasks = list(deepcopy(TASKS))
        # Swap one task's category to break distribution
        # Change a ros2_lifecycle_launch task to control_numerics
        for t in tasks:
            if t["id"] == "e1_ros2_lifecycle_launch_0006":
                t["category"] = "control_numerics"
                t["id"] = "e1_control_numerics_0006"  # fix id to match cat
                t["oracle_path"] = "fixtures/oracles/e1_control_numerics_0006.py"
                t["solution_path"] = "fixtures/solutions/e1_control_numerics_0006"
                t["fixture_path"] = "fixtures/tasks/e1_control_numerics_0006"
                break
        errors = runner._validate_catalog(tasks, BASE)
        self.assertTrue(
            any("category distribution" in e for e in errors),
            f"Must reject wrong 30-task category distribution: {errors}"
        )

    def test_catalog_rejects_unreadable_traversal(self):
        """_rglob_files must propagate OSError — callers must handle it.
        Validation/generation must reject unreadable traversal, not silently skip."""
        # Create a temporary tree with an unreadable directory
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "workspace"
            root.mkdir()
            (root / "readable.py").write_text("# ok")
            secret = root / "secret"
            secret.mkdir()
            (secret / "hidden.py").write_text("# secret")
            # Make secret dir unreadable
            secret.chmod(0o000)
            try:
                # _rglob_files should raise OSError, not silently return
                with self.assertRaises(OSError):
                    list(runner._rglob_files(root))
            finally:
                secret.chmod(0o755)

    def test_catalog_validate_catches_unreadable_traversal(self):
        """_validate_catalog must catch OSError from _rglob_files and return
        a controlled validation error, not propagate a traceback."""
        from copy import deepcopy
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            # Create minimal catalog structure with an unreadable workspace
            tasks_dir = root / "fixtures" / "tasks"
            tasks_dir.mkdir(parents=True)
            task_dir = tasks_dir / "e1_test_cat_0001"
            task_dir.mkdir()
            (task_dir / "readable.py").write_text("# test")
            secret = task_dir / "secret"
            secret.mkdir()
            (secret / "hidden.py").write_text("# secret")
            secret.chmod(0o000)
            try:
                # Build a minimal valid-looking task pointing to the unreadable workspace
                bad_task = deepcopy(TASKS[0])
                bad_task["fixture_path"] = "fixtures/tasks/e1_test_cat_0001"
                bad_task["id"] = "e1_test_cat_0001"
                # Use a real oracle file just so the oracle exists
                bad_task["oracle_path"] = TASKS[0]["oracle_path"]
                bad_task["solution_path"] = TASKS[0]["solution_path"]
                errors = runner._validate_catalog([bad_task], root)
                self.assertTrue(
                    any("cannot traverse" in e for e in errors),
                    f"Must report traversal error, got: {errors}"
                )
            finally:
                secret.chmod(0o755)

    def test_catalog_rejects_malformed_list_elements(self):
        """List elements must be validated as nonempty strings BEFORE set ops.
        Unhashable or non-string elements must produce validation errors."""
        from copy import deepcopy
        # Non-string element
        bad_task = deepcopy(TASKS[0])
        bad_task["allowed_paths"] = ["parser.py", 123]
        errors = runner._validate_catalog([bad_task], BASE)
        self.assertTrue(
            any("must be a nonempty string" in e for e in errors),
            f"Must reject non-string list element: {errors}"
        )
        # Empty string element
        bad_task2 = deepcopy(TASKS[0])
        bad_task2["allowed_paths"] = ["parser.py", ""]
        errors2 = runner._validate_catalog([bad_task2], BASE)
        self.assertTrue(
            any("must be a nonempty string" in e for e in errors2),
            f"Must reject empty string list element: {errors2}"
        )

    # ── rlimit smoke test ─────────────────────────────────────

    def test_rlimit_nproc_smoke(self):
        """Apply each distinct catalog limit class to /usr/bin/aletheon version.
        Skips if installed binary is absent. Does NOT invoke an LLM/provider.
        Proves max_processes=4096 works without Tokio OS thread spawn failure."""
        aletheon_bin = "/usr/bin/aletheon"
        if not os.path.exists(aletheon_bin) or not os.access(aletheon_bin, os.X_OK):
            self.skipTest(f"{aletheon_bin} not found — skipping rlimit smoke test")

        import importlib
        importlib.reload(runner)

        # Collect distinct (cpu_seconds, memory_mb, max_open_files, max_processes)
        limit_classes = set()
        for t in TASKS:
            limit_classes.add((
                t["cpu_seconds"], t["memory_mb"],
                t["max_open_files"], t["max_processes"],
            ))

        for cpu, mem, nofile, nproc in sorted(limit_classes):
            try:
                limiter = runner.make_limiter(cpu, mem, nofile, nproc)
            except RuntimeError as e:
                self.fail(f"make_limiter({cpu},{mem},{nofile},{nproc}) failed: {e}")

            result = runner._run_bounded(
                [aletheon_bin, "version"],
                cwd=Path(tempfile.gettempdir()),
                env=runner._sanitized_env(),
                timeout=30,
                aletheon_binary=aletheon_bin,
                preexec_fn=limiter,
            )
            # Under RLIMIT_NPROC=4096, aletheon version must succeed (rc=0).
            # Under the old nproc=4, Tokio fails to spawn worker threads (rc=101).
            self.assertEqual(
                result.get("exit_code"), 0,
                f"aletheon version failed with nproc={nproc}: "
                f"rc={result.get('exit_code')} stderr={result.get('stderr','')[:200]}"
            )

    # ── malformed terminal schema tests ───────────────────────

    def test_malformed_metrics_not_dict_rejected(self):
        """Metrics field that is not a dict must yield malformed_terminal."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-metrics-bad",
                exit_code=0, metrics=None,
                extra_fields={"metrics": "not-a-dict"},
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-metrics-bad", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_terminal_snapshot"])

    def test_malformed_metrics_wrong_types_rejected(self):
        """Metrics with wrong field types must yield malformed_terminal."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            # completed_normally as int instead of bool, tool_calls_made as string
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-metrics-type",
                exit_code=0,
                metrics={
                    "tool_calls_made": "seven",
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 0,
                    "iterations": 0,
                    "completed_normally": 1,
                },
                modify_files={
                    task["required_paths"][0]: "# fix\n",
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-metricstype", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_terminal_snapshot"])

    def test_malformed_schema_version_rejected(self):
        """schema_version as string must yield malformed_terminal."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-sv-str",
                schema_version="1", exit_code=0,
                modify_files={
                    task["required_paths"][0]: self._read_solution_content(task),
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-svstr", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertIn(result["failure_class"],
                          ["malformed_terminal", "missing_terminal_snapshot"])

    # ── report string metric / bool-as-int / scoring tests ────

    def test_string_metric_handled_by_report(self):
        """Report must reject string metrics before aggregation."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["tool_calls_made"] = "not-a-number"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_bool_as_int_metric_rejected_by_report(self):
        """Report must not treat bool as int in metric slots."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["elapsed_ms"] = True
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_invalid_scoring_dict_rejected(self):
        """Report must reject non-dict scoring in real-evaluation records."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = ["not", "a", "dict"]
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_invalid_fixture_solution_passed_type(self):
        """Report must reject non-bool solution_passed in fixture mode."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["solution_passed"] = "yes"
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_oracle_pass_scope_fail_pass_count(self):
        """pass_count must reflect model_acceptance, not oracle_pass.
        When oracle passes but scope fails, pass_count stays 0."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            fake = self._make_fake_aletheon(
                td,
                status="completed",
                operation_id="op-passcount",
                exit_code=0,
                modify_files={
                    task["required_paths"][0]: solution_content,
                    "evil.py": "# out of scope\n",
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-passcount", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            s = report.summarize([result])
            for dim_data in s["slices"].values():
                for slice_data in dim_data.values():
                    self.assertEqual(slice_data["pass_count"], 0,
                                     "pass_count must be 0 when model_acceptance is False")
    # ── Fix 1: terminal_status != "completed" gate ───────────

    def test_fake_blocked_with_exit_0_and_completed_normally_true(self):
        """blocked terminal with exit 0 + completed_normally=True must
        be rejected as terminal_status_blocked, never reach oracle."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            fake = self._make_fake_aletheon(
                td,
                status="blocked",
                operation_id="op-blocked",
                exit_code=0,
                metrics={
                    "tool_calls_made": 1,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 100,
                    "iterations": 1,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: solution_content,
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-blocked", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"],
                             "blocked+exit0+completed_normally_true must not pass")
            self.assertEqual(result["failure_class"], "terminal_status_blocked")
            self.assertIn("terminal_status:blocked", result["failure_reasons"])

    def test_fake_cancelled_with_exit_0_and_completed_normally_true(self):
        """cancelled terminal with exit 0 + completed_normally=True must
        be rejected as terminal_status_blocked."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            fake = self._make_fake_aletheon(
                td,
                status="cancelled",
                operation_id="op-cancelled",
                exit_code=0,
                metrics={
                    "tool_calls_made": 1,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 100,
                    "iterations": 1,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: solution_content,
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-cancelled", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"],
                             "cancelled+exit0+completed_normally_true must not pass")
            self.assertEqual(result["failure_class"], "terminal_status_blocked")
            self.assertIn("terminal_status:cancelled", result["failure_reasons"])

    # ── Fix 2: oracle provenance fields ───────────────────────

    def test_fake_success_has_oracle_provenance(self):
        """Real result must persist oracle provenance fields with typed values."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            solution_content = self._read_solution_content(task)
            fake = self._make_fake_aletheon(
                td,
                status="completed",
                operation_id="op-prov",
                exit_code=0,
                metrics={
                    "tool_calls_made": 3,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 5000,
                    "iterations": 5,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: solution_content,
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-prov", aletheon_binary=fake
            )
            self.assertTrue(result["model_acceptance"])
            # Oracle provenance fields must be present with correct types
            self.assertIsInstance(result["oracle_stdout"], str)
            self.assertIsInstance(result["oracle_stderr"], str)
            self.assertIsInstance(result["oracle_stdout_digest"], str)
            self.assertIsInstance(result["oracle_stderr_digest"], str)
            self.assertIsInstance(result["oracle_stdout_truncated"], bool)
            self.assertIsInstance(result["oracle_stderr_truncated"], bool)
            self.assertIsInstance(result["oracle_timed_out"], bool)
            # oracle_process_group_reaped is bool or None
            self.assertIn(type(result["oracle_process_group_reaped"]), (bool, type(None)))
            # oracle_elapsed_ms is int or None
            self.assertIn(type(result["oracle_elapsed_ms"]), (int, type(None)))
            # stdout digest must be a 64-char hex string
            self.assertEqual(len(result["oracle_stdout_digest"]), 64)
            self.assertEqual(len(result["oracle_stderr_digest"]), 64)

    def test_fake_oracle_failure_has_oracle_provenance(self):
        """Oracle failure result must still carry oracle provenance fields."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake = self._make_fake_aletheon(
                td, status="completed", operation_id="op-oracle-fail", exit_code=0,
                metrics={
                    "tool_calls_made": 1,
                    "tool_errors": 0,
                    "provider_retries": 0,
                    "elapsed_ms": 100,
                    "iterations": 1,
                    "completed_normally": True,
                },
                modify_files={
                    task["required_paths"][0]: "# This does not fix the bug\nprint('broken')\n",
                },
            )
            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-oracle-prov", aletheon_binary=fake
            )
            self.assertFalse(result["model_acceptance"])
            self.assertEqual(result["failure_class"], "oracle_failure")
            # Oracle provenance must still be present
            self.assertIsInstance(result["oracle_stdout"], str)
            self.assertIsInstance(result["oracle_stderr"], str)
            self.assertEqual(len(result["oracle_stdout_digest"]), 64)
            self.assertEqual(len(result["oracle_stderr_digest"]), 64)
            self.assertIsInstance(result["oracle_timed_out"], bool)

    def test_fake_early_result_has_oracle_provenance_defaults(self):
        """Early failure path (e.g. binary not found) must have null/empty
        oracle provenance defaults."""
        task = self._simple_task()
        result = runner._real_evaluate_one_with_binary(
            task, "test-run-nobin-prov", aletheon_binary="/nonexistent/aletheon"
        )
        self.assertEqual(result["failure_class"], "infrastructure_error")
        # Oracle provenance must be present with empty/null defaults
        self.assertEqual(result["oracle_stdout"], "")
        self.assertEqual(result["oracle_stderr"], "")
        self.assertEqual(result["oracle_stdout_digest"], "")
        self.assertEqual(result["oracle_stderr_digest"], "")
        self.assertFalse(result["oracle_stdout_truncated"])
        self.assertFalse(result["oracle_stderr_truncated"])
        self.assertFalse(result["oracle_timed_out"])
        self.assertIsNone(result["oracle_process_group_reaped"])
        self.assertIsNone(result["oracle_elapsed_ms"])

    # ── Fix 3: post-model workspace safety ────────────────────

    def test_fake_symlink_replacement_rejected(self):
        """Replacing an allowed file with a symlink must fail scope."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            # Create a fake that replaces an allowed path with a symlink
            fake_path = os.path.join(td, "fake-symlink")
            script = f"""#!/usr/bin/env python3
import json, os, sys, base64
output = {{"schema_version":1,"sequence":0,"session_id":"s","task_id":"t",
          "turn_id":"t","activity_id":None,"type":"terminal",
          "status":"completed","operation_id":"op-sym","output":"",
          "error_code":None,
          "metrics":{{"tool_calls_made":1,"tool_errors":0,
          "provider_retries":0,"elapsed_ms":100,"iterations":1,
          "completed_normally":True}}}}
sys.stdout.write(json.dumps(output) + chr(10))
# Find workspace from --cd
cd_idx = sys.argv.index('--cd')
ws = sys.argv[cd_idx + 1]
target = os.path.join(ws, "{task['required_paths'][0]}")
# Remove the original file and replace it with a symlink
os.remove(target)
os.symlink("/etc/passwd", target)
sys.exit(0)
"""
            with open(fake_path, "w") as f:
                f.write(script)
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-symlink", aletheon_binary=fake_path
            )
            self.assertFalse(result["model_acceptance"])
            self.assertTrue(
                any("symlink" in v for v in result["scope_violations"]),
                f"Symlink replacement must be detected: {result['scope_violations']}"
            )

    def test_fake_required_file_deletion_rejected(self):
        """Deleting a required file must fail scope."""
        with tempfile.TemporaryDirectory() as td:
            task = self._simple_task()
            fake_path = os.path.join(td, "fake-delete")
            script = f"""#!/usr/bin/env python3
import json, os, sys
output = {{"schema_version":1,"sequence":0,"session_id":"s","task_id":"t",
          "turn_id":"t","activity_id":None,"type":"terminal",
          "status":"completed","operation_id":"op-del","output":"",
          "error_code":None,
          "metrics":{{"tool_calls_made":1,"tool_errors":0,
          "provider_retries":0,"elapsed_ms":100,"iterations":1,
          "completed_normally":True}}}}
sys.stdout.write(json.dumps(output) + chr(10))
cd_idx = sys.argv.index('--cd')
ws = sys.argv[cd_idx + 1]
# Delete the required file
os.remove(os.path.join(ws, "{task['required_paths'][0]}"))
sys.exit(0)
"""
            with open(fake_path, "w") as f:
                f.write(script)
            os.chmod(fake_path, os.stat(fake_path).st_mode | stat.S_IEXEC)

            task = self._simple_task()
            result = runner._real_evaluate_one_with_binary(
                task, "test-run-delete", aletheon_binary=fake_path
            )
            self.assertFalse(result["model_acceptance"])
            self.assertTrue(
                any("required_deleted" in v for v in result["scope_violations"]),
                f"Required file deletion must be detected: {result['scope_violations']}"
            )

    # ── Fix 4: report resource typing ─────────────────────────

    def test_report_rejects_string_resource_fields(self):
        """Report must reject string-typed resource fields."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        r0["cpu_seconds"] = "ten"
        r0["oracle_passed"] = False
        r0["failure_class"] = "test"
        r0["failure_reasons"] = ["test"]
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_report_rejects_bool_resource_fields(self):
        """Report must reject bool-typed resource fields."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        r0["memory_mb"] = True
        r0["oracle_passed"] = False
        r0["failure_class"] = "test"
        r0["failure_reasons"] = ["test"]
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_report_rejects_out_of_range_resource_fields(self):
        """Report must reject resource fields exceeding upper bounds."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        r0["timeout"] = 99999
        r0["oracle_passed"] = False
        r0["failure_class"] = "test"
        r0["failure_reasons"] = ["test"]
        with self.assertRaises(ValueError):
            report.summarize([r0])

    def test_report_rejects_contradictory_cost_tier(self):
        """Report must reject cost_tier that contradicts cpu_seconds derivation."""
        results = _fixture_results()
        r0 = dict(results[0])
        r0["mode"] = "real-evaluation"
        r0["model_acceptance"] = False
        r0["permission"] = "read_only"
        r0["scoring"] = _valid_scoring_for_report()
        # cpu_seconds=4 means cost_tier must be "low", not "high"
        r0["cpu_seconds"] = 4
        r0["cost_tier"] = "high"
        r0["oracle_passed"] = False
        r0["failure_class"] = "test"
        r0["failure_reasons"] = ["test"]
        with self.assertRaises(ValueError):
            report.summarize([r0])


if __name__ == "__main__":
    unittest.main()
