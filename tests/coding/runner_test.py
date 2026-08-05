#!/usr/bin/env python3
"""Fake-client tests for the real coding benchmark runner boundary."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import pathlib
import signal
import stat
import sys
import tempfile
import textwrap
import threading
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
HARNESS = HERE / "harness"
sys.path.insert(0, str(HARNESS))


def load_module(name: str, path: pathlib.Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


runner = load_module("coding_runner", HARNESS / "run.py")
receipt_contract = load_module("runner_receipt", HARNESS / "receipt.py")


FAKE_CLIENT = r'''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys, time

scenario = os.environ["FAKE_SCENARIO"]
workspace = pathlib.Path(sys.argv[sys.argv.index("--cd") + 1])
if scenario in {"completed", "dirty", "cargo", "leak"}:
    (workspace / "src/lib.rs").write_text("fixed\n")
elif scenario == "false_success":
    (workspace / "src/lib.rs").write_text("wrong\n")
elif scenario == "scope":
    (workspace / "outside.txt").write_text("not allowed\n")
elif scenario == "timeout":
    time.sleep(5)
    raise SystemExit(0)

if scenario == "malformed":
    sys.stdout.write("{not-json")
    raise SystemExit(0)

if scenario == "leak":
    subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(30)"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

stop = "completed"
success = True
operation_id = "op-123"
exit_code = 0
if scenario in {"blocked", "budget"}:
    stop, success = "blocked", False
elif scenario == "failed":
    stop, success, exit_code = "failed", False, 7
elif scenario == "missing_operation":
    operation_id = ""

payload = {
    "success": success,
    "operation_id": operation_id,
    "response": "R" * (70000 if os.environ.get("FAKE_LARGE") else 1),
    "stop": stop,
    "iterations": 1,
    "tool_calls_made": 2,
    "tool_errors": 0 if stop != "failed" else 1,
    "provider_retries": 3,
    "elapsed_ms": 9,
}
encoded = json.dumps(payload, sort_keys=True).encode()
sys.stdout.buffer.write(encoded)
if os.environ.get("FAKE_LARGE"):
    sys.stderr.buffer.write(b"E" * 70000)
raise SystemExit(exit_code)
'''


class RunnerTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        fixture = self.root / "tests/coding/fixtures/sample"
        (fixture / "src").mkdir(parents=True)
        (fixture / "src/lib.rs").write_text("original\n")
        (fixture / "dirty.txt").write_text("original dirty\n")
        (fixture / "Cargo.toml").write_text("[workspace]\n")
        (self.root / "tests/coding/acceptance").mkdir(parents=True)
        (self.root / "scripts").mkdir()
        self.binary = self.root / "fake-aletheon"
        self.binary.write_text(FAKE_CLIENT)
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        self.receipts = self.root / "receipts"

    def tearDown(self):
        self.temp.cleanup()

    def task(
        self,
        *,
        expected: str = "verified",
        required: tuple[str, ...] = ("src/",),
        acceptance: list[list[str]] | None = None,
        setup: dict[str, object] | None = None,
    ) -> pathlib.Path:
        acceptance = acceptance or [
            [
                sys.executable,
                "-c",
                "import pathlib,sys;sys.exit(pathlib.Path('src/lib.rs').read_text()!='fixed\\n')",
            ]
        ]
        setup = setup or {}
        path = self.root / "task.toml"
        fields = [
            "schema_version = 1",
            'id = "sample"',
            'category = "behavioral_bugfix"',
            'fixture = "sample"',
            'prompt = "make the requested repair"',
            "timeout_secs = 1",
            f"acceptance_commands = {json.dumps(acceptance)}",
            'forbidden_paths = ["Cargo.toml"]',
            f"required_changed_paths = {json.dumps(list(required))}",
            f'expected_terminal = "{expected}"',
            'resource_checks = ["no_descendant_processes"]',
            "",
            "[setup]",
        ]
        for key, value in setup.items():
            fields.append(f"{key} = {json.dumps(value)}")
        path.write_text("\n".join(fields) + "\n")
        return path

    def execute(self, scenario: str, task: pathlib.Path, **extra_env: str) -> dict:
        output = self.receipts / f"{scenario}.json"
        env = os.environ.copy()
        env.update(
            {
                "ALETHEON_BIN": str(self.binary),
                "FAKE_SCENARIO": scenario,
                **extra_env,
            }
        )
        value = runner.run_task(task, output, root=self.root, environ=env)
        self.assertTrue(output.is_file())
        self.assertEqual(json.loads(output.read_text()), value)
        self.assertTrue(receipt_contract.verify_integrity(value))
        replayed, message = receipt_contract.verify_receipt(value)
        self.assertEqual(replayed, value["verification"]["passed"])
        self.assertEqual(
            message,
            "verified" if replayed else value["failure"]["class"],
        )
        return value

    def test_installed_terminal_envelope_normalizes_to_runner_contract(self):
        execution = {
            "_stdout_bytes": json.dumps(
                {
                    "schema_version": 1,
                    "type": "terminal",
                    "status": "completed",
                    "operation_id": "op-installed",
                    "metrics": {
                        "iterations": 2,
                        "tool_calls_made": 3,
                        "tool_errors": 0,
                        "provider_retries": 1,
                        "elapsed_ms": 42,
                        "completed_normally": True,
                    },
                }
            ).encode()
        }
        parsed, valid = runner._parse_executive(execution)
        self.assertTrue(valid)
        self.assertEqual(parsed["stop"], "completed")
        self.assertTrue(parsed["success"])
        self.assertEqual(parsed["iterations"], 2)
        self.assertEqual(parsed["tool_calls_made"], 3)
        self.assertEqual(parsed["provider_retries"], 1)

    def test_completed_receipt_bounds_output_and_preserves_full_digests(self):
        value = self.execute("completed", self.task(), FAKE_LARGE="1")
        execution = value["execution"]
        self.assertTrue(value["verification"]["passed"])
        self.assertEqual(value["failure"], {"class": "none", "reasons": []})
        self.assertTrue(execution["stdout_truncated"])
        self.assertTrue(execution["stderr_truncated"])
        self.assertLessEqual(len(execution["stdout"].encode()), runner.MAX_CAPTURE)
        self.assertEqual(
            execution["stderr_digest"],
            "sha256:" + hashlib.sha256(b"E" * 70000).hexdigest(),
        )
        self.assertEqual(value["metrics"]["provider_retries"], 3)
        self.assertIsNone(value["metrics"]["inference_rounds"])
        self.assertIsNone(value["metrics"]["active_context_tokens"])

    def test_default_binary_matches_the_shared_cargo_agent_target(self):
        self.assertEqual(
            runner.default_binary({"HOME": "/tmp/test-home"}),
            pathlib.Path(
                "/tmp/test-home/.cache/aletheon-cargo/target/debug/aletheon"
            ),
        )
        self.assertEqual(
            runner.default_binary({"CARGO_TARGET_DIR": "/tmp/custom-target"}),
            pathlib.Path("/tmp/custom-target/debug/aletheon"),
        )

    def test_caller_interrupt_reaps_the_active_process_group(self):
        child_pid_file = self.root / "active-child.pid"

        def interrupt_after_child_starts():
            deadline = time.monotonic() + 2
            while not child_pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            os.kill(os.getpid(), signal.SIGINT)

        interrupter = threading.Thread(target=interrupt_after_child_starts)
        interrupter.start()
        with self.assertRaises(KeyboardInterrupt):
            runner.run_bounded(
                [
                    sys.executable,
                    "-c",
                    (
                        "import os,pathlib,time;"
                        f"pathlib.Path({str(child_pid_file)!r}).write_text(str(os.getpid()));"
                        "time.sleep(30)"
                    ),
                ],
                self.root,
                os.environ,
                30,
            )
        interrupter.join()
        process_group = int(child_pid_file.read_text())
        with self.assertRaises(ProcessLookupError):
            os.killpg(process_group, 0)

    def test_terminal_and_transport_failures_always_write_receipts(self):
        cases = {
            "blocked": "execution_failure",
            "failed": "execution_failure",
            "malformed": "infrastructure_failure",
            "missing_operation": "infrastructure_failure",
            "timeout": "timeout_or_cancellation",
            "false_success": "verification_failure",
        }
        unchanged = [
            [
                sys.executable,
                "-c",
                "import pathlib,sys;sys.exit(pathlib.Path('src/lib.rs').read_text()!='original\\n')",
            ]
        ]
        for scenario, expected_class in cases.items():
            with self.subTest(scenario=scenario):
                required = () if scenario in {"blocked", "failed", "malformed", "missing_operation", "timeout"} else ("src/",)
                acceptance = unchanged if not required else None
                value = self.execute(
                    scenario,
                    self.task(required=required, acceptance=acceptance),
                )
                self.assertFalse(value["verification"]["passed"])
                self.assertEqual(value["failure"]["class"], expected_class)

    def test_expected_block_and_budget_are_valid_non_success_outcomes(self):
        unchanged = [
            [
                sys.executable,
                "-c",
                "import pathlib,sys;sys.exit(pathlib.Path('src/lib.rs').read_text()!='original\\n')",
            ]
        ]
        blocked = self.execute(
            "blocked",
            self.task(expected="blocked", required=(), acceptance=unchanged),
        )
        self.assertTrue(blocked["verification"]["passed"])

        budget = self.execute(
            "budget",
            self.task(
                expected="budget_exhausted",
                required=(),
                acceptance=unchanged,
                setup={"exec_max_turns": 1},
            ),
        )
        self.assertTrue(budget["verification"]["passed"])
        self.assertIn("--max-turns", budget["execution"]["argv"])

    def test_dirty_setup_is_preserved_and_scope_changes_are_rejected(self):
        dirty = self.execute(
            "dirty",
            self.task(
                setup={"dirty_path": "dirty.txt", "dirty_content": "user change\n"}
            ),
        )
        self.assertTrue(dirty["workspace"]["dirty_patch_preserved"])
        self.assertIn("dirty.txt", dirty["workspace"]["changed_files"])
        self.assertTrue(dirty["verification"]["passed"])

        scope = self.execute("scope", self.task())
        self.assertEqual(scope["failure"]["class"], "policy_scope_failure")
        self.assertIn("outside.txt", scope["workspace"]["changed_files"])

    def test_cargo_is_wrapped_and_remaining_process_group_is_reaped(self):
        wrapper_log = self.root / "wrapper.log"
        wrapper = self.root / "scripts/cargo-agent.sh"
        wrapper.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env bash
                printf '%s\\n%s\\n%s\\n%s\\n' \
                  "$*" "$RUSTUP_HOME" "$CARGO_HOME" "$HOME" > "$WRAP_LOG"
                """
            )
        )
        wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR)
        value = self.execute(
            "leak",
            self.task(acceptance=[["cargo", "test", "--quiet"]]),
            WRAP_LOG=str(wrapper_log),
            HOME=str(self.root / "outer-home"),
        )
        self.assertTrue(value["execution"]["process_group_reaped"])
        self.assertTrue(value["resources"]["passed"])
        self.assertEqual(value["acceptance"][0]["argv"][:2], ["bash", str(wrapper)])
        wrapper_lines = wrapper_log.read_text().splitlines()
        self.assertEqual(wrapper_lines[:3], [
            "test --quiet",
            str(self.root / "outer-home/.rustup"),
            str(self.root / "outer-home/.cargo"),
        ])
        self.assertNotEqual(wrapper_lines[3], str(self.root / "outer-home"))


if __name__ == "__main__":
    unittest.main()
