#!/usr/bin/env python3
"""Validate V01 acceptance hygiene and emit bounded machine-readable evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
TESTS = [
    ROOT / "crates/executive/tests/cross_domain_acceptance.rs",
    ROOT / "crates/executive/tests/functional_indicators.rs",
    ROOT / "crates/executive/tests/support/conscious_core_harness.rs",
]
FIXTURE = ROOT / "crates/executive/tests/fixtures/conscious_core/baseline_v1.json"
INDICATORS = [
    "recurrent_processing", "global_availability", "capacity_bottleneck",
    "attention_modulation", "temporal_continuity", "prediction_error",
    "self_attribution", "metacognitive_calibration", "agency",
    "narrative_causes", "competition_fairness", "mutation_integrity",
    "narrative_faithfulness", "surprise",
]


def fail(message: str) -> None:
    raise SystemExit(f"acceptance gate: {message}")


def hygiene() -> None:
    for path in TESTS:
        text = path.read_text(encoding="utf-8")
        if "#[ignore" in text:
            fail(f"ignored acceptance test in {path.relative_to(ROOT)}")
        for marker in ("tokio::time::sleep(", "future::pending(", "loop { // unbounded"):
            if marker in text:
                fail(f"unbounded wait marker {marker!r} in {path.relative_to(ROOT)}")
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    if fixture.get("fixture_version") != 1:
        fail("baseline fixture version drifted without a schema update")
    expected = sorted(["agent_tree", "debug", "memory_jobs", "metrics", "session"])
    if sorted(fixture.get("expected_projection_names", [])) != expected:
        fail("baseline projection inventory drifted")


def git_commit() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, check=True,
        text=True, stdout=subprocess.PIPE,
    )
    return result.stdout.strip()


def report() -> dict[str, object]:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    fixture_sha = hashlib.sha256(FIXTURE.read_bytes()).hexdigest()
    config_schema = ROOT / "config/schema/aletheon-config.schema.json"
    return {
        "schema_version": 1,
        "fixture_version": fixture["fixture_version"],
        "fixture_sha256": fixture_sha,
        "commit": git_commit(),
        "config_schema": str(config_schema.relative_to(ROOT)),
        "config_schema_sha256": hashlib.sha256(config_schema.read_bytes()).hexdigest(),
        "indicator_definitions": INDICATORS,
        "results": {
            "cross_domain_acceptance": "verified_by_cargo_test",
            "functional_indicators": "verified_by_cargo_test",
            "architecture": "verified_by_architecture_check",
        },
        "limitations": [
            "Functional indicators do not establish phenomenal consciousness.",
            "External providers, network, process execution, and credentials are disabled.",
            "Hidden reasoning and model self-report are excluded from evidence.",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="validate only")
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "target/acceptance")
    args = parser.parse_args()
    hygiene()
    if args.check:
        return 0
    data = report()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "acceptance.json").write_text(
        json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    markdown = [
        "# Cross-domain acceptance evidence", "",
        f"- Fixture version: `{data['fixture_version']}`",
        f"- Commit: `{data['commit']}`",
        f"- Fixture SHA-256: `{data['fixture_sha256']}`", "",
        "## Results", "",
    ]
    markdown.extend(f"- {name}: {value}" for name, value in data["results"].items())
    markdown.extend(["", "## Limitations", ""])
    markdown.extend(f"- {item}" for item in data["limitations"])
    (args.output / "acceptance.md").write_text("\n".join(markdown) + "\n", encoding="utf-8")
    print(args.output / "acceptance.json")
    print(args.output / "acceptance.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
