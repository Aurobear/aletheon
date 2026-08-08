from src.lab.store import LabStore


def _result(run_id, finished_at, fingerprint="sha256:abc"):
    return {
        "run_id": run_id,
        "case_id": "smoke.v1",
        "source": {"commit_sha": "a" * 40},
        "started_at": "2026-08-07T00:00:00Z",
        "finished_at": finished_at,
        "outcome": "product_failed" if fingerprint else "passed",
        "failure_class": "command_failure" if fingerprint else "none",
        **({"fingerprint": fingerprint} if fingerprint else {}),
    }


def test_store_clusters_repeated_occurrences_and_survives_reopen(tmp_path):
    path = tmp_path / "state" / "lab.sqlite"
    with LabStore(path) as store:
        first = store.record(
            _result("run-1", "2026-08-07T00:01:00Z"), tmp_path / "run-1"
        )
        second = store.record(
            _result("run-2", "2026-08-07T00:02:00Z"), tmp_path / "run-2"
        )
        assert first["is_new"] is True
        assert first["occurrence_count"] == 1
        assert second["is_new"] is False
        assert second["occurrence_count"] == 2

    with LabStore(path) as reopened:
        cluster = reopened.cluster("sha256:abc")
        assert cluster["occurrence_count"] == 2
        assert cluster["latest_run_id"] == "run-2"
        assert reopened.latest_finished_at("smoke.v1") == "2026-08-07T00:02:00Z"


def test_passing_run_does_not_create_failure_cluster(tmp_path):
    with LabStore(tmp_path / "lab.sqlite") as store:
        recorded = store.record(
            _result("run-pass", "2026-08-07T00:01:00Z", fingerprint=None),
            tmp_path / "run-pass",
        )
        assert recorded == {
            "is_new": False,
            "occurrence_count": 0,
            "fingerprint": None,
        }


def test_diagnostic_daily_budget_is_reserved_atomically(tmp_path):
    with LabStore(tmp_path / "lab.sqlite") as store:
        assert store.reserve_diagnostic_call(
            run_id="run-1",
            requested_at="2026-08-07T00:00:00Z",
            max_calls_per_day=1,
        )
        assert not store.reserve_diagnostic_call(
            run_id="run-2",
            requested_at="2026-08-07T01:00:00Z",
            max_calls_per_day=1,
        )
        assert store.reserve_diagnostic_call(
            run_id="run-3",
            requested_at="2026-08-08T00:00:00Z",
            max_calls_per_day=1,
        )
