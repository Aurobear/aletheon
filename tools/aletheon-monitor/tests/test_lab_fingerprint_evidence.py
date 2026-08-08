import json

import pytest
from src.lab.evidence import EvidenceBundle
from src.lab.fingerprint import failure_fingerprint, normalize_signature


def test_dynamic_failure_values_do_not_split_a_cluster():
    first = failure_fingerprint(
        case_id="turn.scope.v1",
        failure_class="command_failure",
        signature=(
            "2026-08-07T12:10:11Z pid=124 operation_id="
            "3fa85f64-5717-4562-b3fc-2c963f66afa6 /tmp/run-a/output"
        ),
        invariant_ids=["scope_drained"],
    )
    second = failure_fingerprint(
        case_id="turn.scope.v1",
        failure_class="command_failure",
        signature=(
            "2026-08-08T01:02:03Z pid=999 operation_id="
            "9b2d7d85-23a9-4c0f-a6fd-2ce512d7db20 /tmp/run-b/output"
        ),
        invariant_ids=["scope_drained"],
    )
    assert first == second


def test_different_invariant_produces_different_fingerprint():
    common = {
        "case_id": "turn.scope.v1",
        "failure_class": "command_failure",
        "signature": "assertion failed",
    }
    assert failure_fingerprint(
        **common, invariant_ids=["scope_drained"]
    ) != failure_fingerprint(**common, invariant_ids=["settlement_valid"])


def test_signature_normalization_is_bounded():
    assert len(normalize_signature("A" * 20_000)) == 8192


def test_bundle_is_append_only_then_sealed(tmp_path):
    bundle = EvidenceBundle(tmp_path, "run-001")
    bundle.write_json("manifest.json", {"schema_version": 1})
    bundle.append_event("run_started", {"case_id": "smoke.v1"})
    bundle.write_json("result.json", {"outcome": "passed"})

    receipt_path = bundle.seal()
    receipt = json.loads(receipt_path.read_text())
    assert receipt["schema_version"] == 1
    assert {item["name"] for item in receipt["files"]} == {
        "manifest.json",
        "result.json",
        "supervisor-events.jsonl",
    }
    assert all(item["sha256"].startswith("sha256:") for item in receipt["files"])
    with pytest.raises(RuntimeError, match="sealed"):
        bundle.write_json("late.json", {})


def test_bundle_rejects_path_traversal(tmp_path):
    bundle = EvidenceBundle(tmp_path, "run-001")
    with pytest.raises(ValueError, match="simple"):
        bundle.write_json("../escape.json", {})
