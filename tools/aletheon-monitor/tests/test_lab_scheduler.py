from types import SimpleNamespace

from src.lab.model import CaseSpec
from src.lab.scheduler import Nightwatch


def test_cycle_records_scheduler_errors_and_cannot_report_clean_success():
    nightwatch = Nightwatch.__new__(Nightwatch)
    nightwatch.settings = SimpleNamespace(
        cases=(CaseSpec(case_id="failing.v1", command=("true",)),)
    )
    nightwatch.stop_requested = False
    nightwatch.last_cycle_error_count = 0
    recorded = []
    nightwatch._due = lambda *_: True
    nightwatch._emit = lambda event, **fields: recorded.append((event, fields))

    def fail(_case_id):
        raise RuntimeError("source unavailable")

    nightwatch.run_case = fail
    results = Nightwatch.cycle(nightwatch, force=True)

    assert results == []
    assert nightwatch.last_cycle_error_count == 1
    assert recorded == [
        (
            "scheduler_error",
            {"case_id": "failing.v1", "error": "RuntimeError: source unavailable"},
        )
    ]
