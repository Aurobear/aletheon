#!/usr/bin/env python3
"""Validate V01 acceptance hygiene and emit bounded machine-readable evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
TESTS = [
    ROOT / "crates/executive/tests/cross_domain_acceptance.rs",
    ROOT / "crates/executive/tests/functional_indicators.rs",
    ROOT / "crates/executive/tests/support/conscious_core_harness.rs",
]
FIXTURE = ROOT / "crates/executive/tests/fixtures/conscious_core/baseline_v1.json"
RUNTIME_EVIDENCE = ROOT / "target/acceptance/runtime-evidence.json"
INDICATOR_EVIDENCE = ROOT / "target/acceptance/indicator-evidence.json"
ABLATION_EVIDENCE = ROOT / "target/acceptance/ablation-evidence.json"
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


def load_receipt(path: pathlib.Path) -> dict[str, object]:
    if not path.is_file():
        fail(f"missing test receipt {path.relative_to(ROOT)}")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        fail(f"invalid schema in test receipt {path.relative_to(ROOT)}")
    return value


def validate_receipts() -> tuple[dict[str, object], dict[str, object], dict[str, object]]:
    runtime = load_receipt(RUNTIME_EVIDENCE)
    indicators = load_receipt(INDICATOR_EVIDENCE)
    ablations = load_receipt(ABLATION_EVIDENCE)
    checksum = re.compile(r"[0-9a-f]{64}").fullmatch

    if runtime.get("fixture_version") != 1:
        fail("runtime receipt fixture version drifted")
    if not isinstance(runtime.get("event_checksum"), str) or not checksum(runtime["event_checksum"]):
        fail("runtime receipt has an invalid event checksum")
    projections = runtime.get("projection_checksums")
    expected_projections = {"agent_tree", "debug", "memory_jobs", "metrics", "session"}
    if not isinstance(projections, dict) or set(projections) != expected_projections:
        fail("runtime receipt projection inventory drifted")
    if any(not isinstance(value, str) or not checksum(value) for value in projections.values()):
        fail("runtime receipt has an invalid projection checksum")
    expected_runtime = {
        "replayed_from_independent_root": True,
        "agent_runs_reopened": 2,
        "mailbox_deliveries_reopened": 2,
        "memory_lease_recovered": True,
        "unexpected_external_calls": 0,
    }
    for name, expected in expected_runtime.items():
        if runtime.get(name) != expected:
            fail(f"runtime receipt failed {name}: expected {expected!r}")

    measured = indicators.get("indicators")
    if not isinstance(measured, list):
        fail("indicator receipt does not contain measurements")
    names = [item.get("name") for item in measured if isinstance(item, dict)]
    if len(names) != len(measured) or set(names) != set(INDICATORS) or len(names) != len(INDICATORS):
        fail("indicator receipt inventory drifted")
    if any(item.get("passed") is not True for item in measured):
        fail("one or more functional indicators failed")

    measured_ablations = ablations.get("ablations")
    if not isinstance(measured_ablations, dict) or set(measured_ablations) != {"workspace", "recurrence", "dasein"}:
        fail("ablation receipt inventory drifted")
    for name, measurement in measured_ablations.items():
        if not isinstance(measurement, dict):
            fail(f"ablation receipt {name} is malformed")
        baseline = measurement.get("baseline")
        ablated = measurement.get("ablated")
        if not isinstance(baseline, (int, float)) or not isinstance(ablated, (int, float)) or baseline <= ablated:
            fail(f"ablation {name} did not reduce its target metric")
    return runtime, indicators, ablations


def report() -> dict[str, object]:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    runtime, indicators, ablations = validate_receipts()
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
            "cross_domain_acceptance": {
                "event_checksum": runtime["event_checksum"],
                "projection_checksums": runtime["projection_checksums"],
                "agent_runs_reopened": runtime["agent_runs_reopened"],
                "mailbox_deliveries_reopened": runtime["mailbox_deliveries_reopened"],
                "memory_lease_recovered": runtime["memory_lease_recovered"],
                "unexpected_external_calls": runtime["unexpected_external_calls"],
            },
            "functional_indicators": indicators["indicators"],
            "ablations": ablations["ablations"],
            "architecture_gate": {
                "command": "just architecture-check",
                "status": "required_by_acceptance_recipe_not_inferred_from_test_receipts",
            },
        },
        "limitations": sorted(set(runtime.get("limitations", [])) | set(indicators.get("limitations", []))),
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
    markdown.extend(
        f"- {name}: `{json.dumps(value, sort_keys=True)}`"
        for name, value in data["results"].items()
    )
    markdown.extend(["", "## Limitations", ""])
    markdown.extend(f"- {item}" for item in data["limitations"])
    (args.output / "acceptance.md").write_text("\n".join(markdown) + "\n", encoding="utf-8")
    print(args.output / "acceptance.json")
    print(args.output / "acceptance.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
