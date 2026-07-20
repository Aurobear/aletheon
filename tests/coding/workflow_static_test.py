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


if __name__ == "__main__":
    unittest.main()
