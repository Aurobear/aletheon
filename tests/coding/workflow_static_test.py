#!/usr/bin/env python3
"""Static contracts for coding-related GitHub Actions workflows."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


class OrdinaryCiWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/ci.yml").read_text()

    def test_has_deterministic_coding_evidence_job(self) -> None:
        self.assertIn("name: Deterministic coding evidence", self.workflow)
        self.assertIn("python3 tests/coding/replay_test.py", self.workflow)
        self.assertIn("bash tests/coding/static_test.sh", self.workflow)

    def test_has_linux_platform_contract_job(self) -> None:
        self.assertIn("name: Linux Platform contract", self.workflow)
        self.assertIn(
            "bash scripts/cargo-agent.sh test -p platform --test contract_suite",
            self.workflow,
        )

    def test_remains_secret_free(self) -> None:
        self.assertNotIn("LEJU_API_KEY", self.workflow)


class RealEvaluationWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/coding-e2e.yml").read_text()

    def test_is_manual_only(self) -> None:
        self.assertIn("  workflow_dispatch:", self.workflow)
        self.assertNotIn("  push:", self.workflow)
        self.assertNotIn("  pull_request:", self.workflow)
        self.assertNotIn("  schedule:", self.workflow)

    def test_pins_provider_model_and_secret_source(self) -> None:
        self.assertIn("ALETHEON_PROVIDER: leju", self.workflow)
        self.assertIn("ALETHEON_MODEL: deepseek/deepseek-v4-pro", self.workflow)
        self.assertIn("LEJU_API_KEY: ${{ secrets.LEJU_API_KEY }}", self.workflow)
        self.assertNotIn("echo $LEJU_API_KEY", self.workflow)
        self.assertNotIn("echo \"$LEJU_API_KEY\"", self.workflow)

    def test_runs_all_fixtures_and_preserves_artifacts(self) -> None:
        for task in ("rust_bugfix", "rust_multifile", "rust_diagnosis"):
            self.assertIn(task, self.workflow)
        self.assertIn("actions/upload-artifact@v4", self.workflow)
        self.assertIn("if: always()", self.workflow)

    def test_preserves_runner_toolchain_under_isolated_home(self) -> None:
        self.assertIn('export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"', self.workflow)
        self.assertIn('export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"', self.workflow)

    def test_completed_operations_are_classified_by_verification(self) -> None:
        self.assertNotIn("or executive_exit != 0", self.workflow)
        self.assertIn('if not receipt.get("operation_id"):', self.workflow)
        self.assertIn('elif not receipt.get("verification", {}).get("passed"):', self.workflow)


if __name__ == "__main__":
    unittest.main()
