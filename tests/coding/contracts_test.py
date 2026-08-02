#!/usr/bin/env python3
from __future__ import annotations

import copy
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "harness"))
from contracts import BenchmarkTask, ContractError, load_catalog  # noqa: E402


VALID = {
    "schema_version": 1,
    "id": "rust_bugfix",
    "category": "behavioral_bugfix",
    "fixture": "rust_bugfix",
    "prompt": "repair the defect",
    "timeout_secs": 300,
    "acceptance_commands": [["cargo", "test", "--quiet"]],
    "forbidden_paths": ["Cargo.toml"],
    "required_changed_paths": ["src/"],
    "expected_terminal": "verified",
    "setup": {},
    "resource_checks": ["no_descendant_processes"],
}


class BenchmarkTaskTest(unittest.TestCase):
    def task(self, value=None):
        return BenchmarkTask.from_mapping(value or copy.deepcopy(VALID), Path("task.toml"))

    def assert_invalid(self, mutate):
        value = copy.deepcopy(VALID)
        mutate(value)
        with self.assertRaises(ContractError):
            self.task(value)

    def test_valid_contract_is_immutable_and_normalized(self):
        task = self.task()
        self.assertEqual(task.required_changed_paths, ("src/",))
        with self.assertRaises(Exception):
            task.id = "changed"

    def test_rejects_missing_unknown_and_unsupported_fields(self):
        self.assert_invalid(lambda value: value.pop("category"))
        self.assert_invalid(lambda value: value.update(unknown=True))
        self.assert_invalid(lambda value: value.update(schema_version=2))
        self.assert_invalid(lambda value: value.update(category="other"))
        self.assert_invalid(lambda value: value.update(expected_terminal="passed"))

    def test_rejects_unsafe_and_overlapping_paths(self):
        for path in ("../outside", "/absolute", "src/../secret"):
            self.assert_invalid(lambda value, path=path: value.update(forbidden_paths=[path]))
        self.assert_invalid(lambda value: value.update(forbidden_paths=["src/lib.rs"]))

    def test_rejects_invalid_commands_deadlines_setup_and_resources(self):
        self.assert_invalid(lambda value: value.update(acceptance_commands=[]))
        self.assert_invalid(lambda value: value.update(acceptance_commands=[[]]))
        self.assert_invalid(lambda value: value.update(timeout_secs=0))
        self.assert_invalid(lambda value: value.update(setup={"dirty_path": "src/lib.rs"}))
        self.assert_invalid(lambda value: value.update(setup={"exec_max_turns": 0}))
        self.assert_invalid(lambda value: value.update(resource_checks=["unknown"]))

    def test_catalog_rejects_duplicate_ids_and_sorts(self):
        root = Path(__file__).resolve().parents[2]
        paths = [root / "tests/coding/tasks/rust_multifile.toml", root / "tests/coding/tasks/rust_bugfix.toml"]
        tasks = load_catalog(paths, root)
        self.assertEqual([task.id for task in tasks], ["rust_bugfix", "rust_multifile"])
        with self.assertRaises(ContractError):
            load_catalog([paths[0], paths[0]], root)


if __name__ == "__main__":
    unittest.main()
