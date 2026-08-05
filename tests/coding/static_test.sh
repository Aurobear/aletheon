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

root = pathlib.Path(sys.argv[1])
sys.path.insert(0, str(root / "tests/coding/harness"))
from contracts import CATEGORIES, load_catalog

tasks = load_catalog(sorted((root / "tests/coding/tasks").glob("*.toml")), root)
expected = {
    "approval_blocked_patch", "budget_exhaustion", "clippy_cleanup",
    "config_schema_sync", "dirty_workspace_preservation", "rust_bugfix",
    "rust_diagnosis", "rust_multifile", "rust_regression_test", "rustdoc_contract",
    "option_default", "retry_backoff", "csv_fields", "saturating_sum", "state_transition",
    "path_normalize", "error_classification", "window_bounds", "canonical_key", "timeout_default",
}
assert {task.id for task in tasks} == expected
assert len(tasks) == 20
assert {task.category for task in tasks} == CATEGORIES
for task in tasks:
    fixture = root / "tests/coding/fixtures" / task.fixture
    assert (fixture / "Cargo.toml").is_file(), task.id
    assert (fixture / "Cargo.lock").is_file(), task.id
    assert "[workspace]" in (fixture / "Cargo.toml").read_text(), task.id
PY

echo 'coding harness static verification: pass'
