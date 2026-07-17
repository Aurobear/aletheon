import asyncio
import json

from scenarios import gmail_analysis
from src import scenarios


ACCOUNT = "production-test@example.invalid"


def event(kind, **params):
    return {"type": kind, "params": {"type": kind, **params}}


def gmail_events(output, *, account=ACCOUNT, is_error=False):
    args = {"account": account, "query": "newer_than:7d", "page_size": 10}
    return [
        event("tool_call_start", call_id="gmail-1", tool="google_gmail_search", args=args),
        event("tool_call_result", call_id="gmail-1", tool="google_gmail_search",
              output=output, is_error=is_error),
    ]


def page(count=2):
    return json.dumps({
        "account_id": "7ae13559-e525-45f7-8467-181f7c826928",
        "messages": [
            {"subject": f"secret-subject-{index}", "from": f"sender-{index}@example.com",
             "snippet": f"private-snippet-{index}", "unread": True, "important": False,
             "thread_id": f"thread-{index}", "source": {}}
            for index in range(count)
        ],
        "next_page_token": None,
    })


def serialized(value):
    return json.dumps(value, sort_keys=True)


def test_authorized_evidence_is_bounded_structured_and_redacted():
    result = scenarios._safe_gmail_evidence(
        gmail_events(page()), "已汇总 2 项邮件元数据。", ACCOUNT, True
    )
    assert result["status"] == "PASS"
    assert result["authorization_state"] == "authorized"
    assert result["evidence"]["item_count"] == 2
    assert result["evidence"]["item_limit"] == 10
    assert result["evidence"]["account_binding"] == "configured_test_account"
    assert result["evidence"]["configured_account_bound"] is True
    report = serialized(result)
    for secret in (ACCOUNT, "secret-subject", "sender-", "private-snippet", "messages"):
        assert secret not in report
    for forbidden in ("body_text", "raw_payload", '"output"', '"frame"'):
        assert forbidden not in report


def test_unauthorized_and_degraded_results_fail_closed_without_payloads():
    unauthorized = scenarios._safe_gmail_evidence(
        gmail_events("google_unauthorized_account", is_error=True),
        "授权失败。", ACCOUNT, True,
    )
    assert unauthorized["status"] == "FAIL"
    assert unauthorized["authorization_state"] == "unauthorized"
    assert unauthorized["failure"] == "google_unauthorized_account"
    assert ACCOUNT not in serialized(unauthorized)

    degraded = scenarios._safe_gmail_evidence(
        gmail_events("google_provider_unavailable", is_error=True),
        "服务暂不可用。", ACCOUNT, True,
    )
    assert degraded["status"] == "FAIL"
    assert degraded["authorization_state"] == "degraded"
    assert degraded["failure"] == "gmail_provider_degraded"
    assert "google_provider_unavailable" not in serialized(degraded)


def test_wrong_account_overflow_and_raw_body_are_rejected():
    wrong = scenarios._safe_gmail_evidence(
        gmail_events(page(), account="some-other-account"), "汇总。", ACCOUNT, True
    )
    assert wrong["status"] == "FAIL"
    assert wrong["authorization_state"] == "degraded"
    assert next(a for a in wrong["assertions"] if a["name"] == "configured_account_bound")["passed"] is False

    overflow = scenarios._safe_gmail_evidence(
        gmail_events(page(11)), "汇总。", ACCOUNT, True
    )
    assert overflow["status"] == "FAIL"
    assert overflow["evidence"]["item_count"] == 11

    raw = json.dumps({"messages": [{"body_text": "raw-secret"}]})
    raw_result = scenarios._safe_gmail_evidence(
        gmail_events(raw), "汇总。", ACCOUNT, True
    )
    assert raw_result["status"] == "FAIL"
    assert raw_result["evidence"]["metadata_only"] is False
    assert "raw-secret" not in serialized(raw_result)


def test_account_in_private_frame_is_redacted_from_report_and_size_is_bounded():
    echoed = scenarios._safe_gmail_evidence(
        gmail_events(page()), f"summary for {ACCOUNT}", ACCOUNT, True
    )
    assert echoed["status"] == "PASS"
    assert ACCOUNT not in serialized(echoed)

    oversized = scenarios._safe_gmail_evidence(
        gmail_events(page()), "x" * (scenarios._GMAIL_MAX_SUMMARY_BYTES + 1), ACCOUNT, True
    )
    assert oversized["status"] == "FAIL"
    assert oversized["evidence"]["summary_bytes"] > oversized["evidence"]["summary_limit_bytes"]


def test_production_scenario_blocks_without_test_account(monkeypatch, tmp_path):
    monkeypatch.delenv("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", raising=False)
    result = asyncio.run(gmail_analysis.run(str(tmp_path)))
    assert result["status"] == "BLOCKED"
    assert result["authorization_state"] == "degraded"
    assert result["failure"] == "gmail_test_account_not_configured"


def test_production_scenario_passes_account_only_to_helper(monkeypatch, tmp_path):
    monkeypatch.setenv("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", ACCOUNT)
    observed = {}

    async def fake_read(source_root, account, timeout):
        observed.update(source_root=source_root, account=account, timeout=timeout)
        return {
            "status": "PASS",
            "authorization_state": "authorized",
            "assertions": [{"name": "authorized", "passed": True}],
            "evidence": {
                "account_binding": "configured_test_account",
                "configured_account_bound": True,
                "item_count": 0,
            },
            "failure": None,
        }

    monkeypatch.setattr(gmail_analysis.base, "gmail_read", fake_read)
    result = asyncio.run(gmail_analysis.run(str(tmp_path), timeout=7))
    assert observed["account"] == ACCOUNT
    assert result["status"] == "PASS"
    assert ACCOUNT not in serialized(result)
