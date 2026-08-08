#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/libexec/aletheon/test-changed.py"
SPEC = importlib.util.spec_from_file_location("test_changed", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def package(root: Path, name: str, dependencies=(), tests=(), target_kind="lib"):
    package_root = root / "crates" / name
    return {
        "id": f"path+file://{package_root}#{name}@0.1.0",
        "name": name,
        "manifest_path": str(package_root / "Cargo.toml"),
        "dependencies": [{"name": dependency} for dependency in dependencies],
        "targets": [
            {
                "name": name,
                "kind": [target_kind],
                "src_path": str(
                    package_root
                    / "src"
                    / ("lib.rs" if target_kind == "lib" else "main.rs")
                ),
            }
        ]
        + [
            {
                "name": target,
                "kind": ["test"],
                "src_path": str(package_root / "tests" / f"{target}.rs"),
            }
            for target in tests
        ],
    }


class ChangedValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "crates/core/tests").mkdir(parents=True)
        (self.root / "crates/client/tests").mkdir(parents=True)
        core = package(self.root, "core", tests=("contract", "runtime"))
        client = package(self.root, "client", dependencies=("core",))
        self.metadata = {
            "packages": [core, client],
            "workspace_members": [core["id"], client["id"]],
        }

    def tearDown(self):
        self.temp.cleanup()

    def commands(self, paths):
        return [step.command for step in MODULE.derive_steps(self.root, paths, self.metadata)]

    def test_source_change_uses_lib_test_and_direct_dependent_check(self):
        commands = self.commands(["crates/core/src/lib.rs"])
        self.assertIn(("bash", "scripts/cargo-agent.sh", "check", "-p", "core"), commands)
        self.assertIn(("bash", "scripts/cargo-agent.sh", "test", "-p", "core", "--lib"), commands)
        self.assertIn(("bash", "scripts/cargo-agent.sh", "check", "-p", "client"), commands)
        self.assertNotIn(("bash", "scripts/cargo-agent.sh", "test", "-p", "core", "--tests"), commands)

    def test_binary_only_source_change_uses_bin_tests(self):
        binary = package(self.root, "runner", target_kind="bin")
        metadata = {"packages": [binary], "workspace_members": [binary["id"]]}
        commands = [
            step.command
            for step in MODULE.derive_steps(
                self.root, ["crates/runner/src/main.rs"], metadata
            )
        ]
        self.assertIn(
            ("bash", "scripts/cargo-agent.sh", "test", "-p", "runner", "--bins"),
            commands,
        )
        self.assertNotIn(
            ("bash", "scripts/cargo-agent.sh", "test", "-p", "runner", "--lib"),
            commands,
        )

    def test_deleted_paths_are_included_in_the_diff(self):
        deleted = self.root / "crates/core/src/lib.rs"
        deleted.parent.mkdir(parents=True, exist_ok=True)
        deleted.write_text("pub fn value() {}\n", encoding="utf-8")
        MODULE.subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        MODULE.subprocess.run(
            ["git", "config", "user.email", "test@example.com"],
            cwd=self.root,
            check=True,
        )
        MODULE.subprocess.run(
            ["git", "config", "user.name", "Test"], cwd=self.root, check=True
        )
        MODULE.subprocess.run(["git", "add", "."], cwd=self.root, check=True)
        MODULE.subprocess.run(
            ["git", "commit", "-qm", "baseline"], cwd=self.root, check=True
        )
        base = MODULE.git_lines(self.root, "rev-parse", "HEAD")[0]
        deleted.unlink()

        self.assertEqual(MODULE.changed_paths(self.root, base), ["crates/core/src/lib.rs"])

    def test_changed_integration_entry_selects_only_that_target(self):
        commands = self.commands(["crates/core/tests/contract.rs"])
        self.assertIn(
            ("bash", "scripts/cargo-agent.sh", "test", "-p", "core", "--test", "contract"),
            commands,
        )
        self.assertNotIn(
            ("bash", "scripts/cargo-agent.sh", "test", "-p", "core", "--test", "runtime"),
            commands,
        )

    def test_shared_integration_support_selects_package_tests(self):
        commands = self.commands(["crates/core/tests/support/mod.rs"])
        self.assertIn(("bash", "scripts/cargo-agent.sh", "test", "-p", "core", "--tests"), commands)

    def test_script_change_selects_static_script_checks(self):
        commands = self.commands(["scripts/lib/aletheon/test.sh"])
        self.assertIn(("bash", "tests/suites/operations/cli_static_test.sh"), commands)
        self.assertIn(("bash", "tests/suites/operations/script_surface_test.sh"), commands)

    def test_failed_rerun_is_intersected_with_current_plan(self):
        steps = MODULE.derive_steps(
            self.root, ["crates/core/src/lib.rs"], self.metadata
        )
        failed = steps[1]
        report = self.root / "report.json"
        payload = {
            "selected_steps": [{"command": list(step.command)} for step in steps],
            "results": [
                {"command": list(steps[0].command), "status": "passed"},
                {"command": list(failed.command), "status": "failed"},
            ],
        }
        report.write_text(json.dumps(payload), encoding="utf-8")
        self.assertEqual(MODULE.select_report_steps(payload, steps, True), [failed])
        self.assertEqual(MODULE.select_report_steps(payload, steps, False), steps[1:])

    def test_stale_failed_command_is_rejected(self):
        steps = MODULE.derive_steps(
            self.root, ["crates/core/src/lib.rs"], self.metadata
        )
        report = self.root / "report.json"
        report.write_text(
            json.dumps(
                {
                    "results": [
                        {"command": ["bash", "unknown.sh"], "status": "failed"}
                    ]
                }
            ),
            encoding="utf-8",
        )
        payload = json.loads(report.read_text(encoding="utf-8"))
        with self.assertRaises(ValueError):
            MODULE.select_report_steps(payload, steps, True)


if __name__ == "__main__":
    unittest.main()
