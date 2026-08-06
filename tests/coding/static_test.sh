#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)

python3 "$root/tests/coding/contracts_test.py"
python3 "$root/tests/coding/runner_test.py"
python3 "$root/tests/coding/replay_test.py"
python3 "$root/tests/coding/suite_test.py"
python3 "$root/tests/coding/workflow_static_test.py"

python3 - <<'PY' "$root"
import pathlib
import sys
import tomllib
from collections import Counter

root = pathlib.Path(sys.argv[1])
sys.path.insert(0, str(root / "tests/coding/harness"))
from contracts import CATEGORIES, load_catalog

tasks = load_catalog(sorted((root / "tests/coding/tasks").glob("*.toml")), root)
expected = {
    "api_error_mapping",
    "approval_blocked_patch",
    "budget_exhaustion",
    "child_orphan_reconciliation",
    "clippy_cleanup",
    "config_precedence_explanation",
    "config_schema_sync",
    "dirty_workspace_preservation",
    "error_flow_explanation",
    "failing_snapshot_test",
    "module_path_explanation",
    "parser_domain_sync",
    "rust_bugfix",
    "rust_diagnosis",
    "rust_multifile",
    "rust_regression_test",
    "rustdoc_contract",
    "session_resume_no_side_effect",
    "timeout_saturation_bugfix",
    "unicode_boundary_bugfix",
}
assert {task.id for task in tasks} == expected
assert len(tasks) == 20
assert {task.category for task in tasks} == CATEGORIES
assert {
    path.name for path in (root / "tests/coding/fixtures").iterdir() if path.is_dir()
} == expected
assert {
    path.name for path in (root / "tests/coding/acceptance").iterdir() if path.is_dir()
} == expected
scenario_counts = Counter()
for task in tasks:
    fixture = root / "tests/coding/fixtures" / task.fixture
    hidden = root / "tests/coding/acceptance" / task.id
    rubric_path = hidden / "rubric.toml"
    assert (fixture / "Cargo.toml").is_file(), task.id
    assert (fixture / "Cargo.lock").is_file(), task.id
    assert "[workspace]" in (fixture / "Cargo.toml").read_text(), task.id
    assert hidden.is_dir(), task.id
    assert any(path.is_file() for path in (hidden / "tests").glob("*.rs")), task.id
    rubric = tomllib.loads(rubric_path.read_text())
    assert set(rubric) == {
        "schema_version", "task_id", "scenario_class", "points_total", "criteria"
    }, task.id
    assert rubric["schema_version"] == 1 and rubric["task_id"] == task.id
    assert rubric["points_total"] == 100
    criteria = rubric["criteria"]
    assert len(criteria) == 4
    assert len({item["id"] for item in criteria}) == len(criteria)
    assert sum(item["weight"] for item in criteria) == rubric["points_total"]
    assert all(item["evidence"].strip() for item in criteria)
    scenario_counts[rubric["scenario_class"]] += 1

assert scenario_counts == {
    "location_explanation": 5,
    "small_bug_fix": 5,
    "cross_file_change": 4,
    "failing_test_repair": 2,
    "review_finding_repair": 2,
    "session_resume": 1,
    "child_orphan_reconciliation": 1,
}
PY

echo 'coding harness static verification: pass'
