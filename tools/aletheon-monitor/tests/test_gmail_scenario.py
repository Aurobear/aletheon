import asyncio
import json

from scenarios import gmail_analysis
from src import scenarios


ACCOUNT = "production-test@example.invalid"
ACCOUNT_ID = "7ae13559-e525-45f7-8467-181f7c826928"


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
        "account_id": ACCOUNT_ID,
        "messages": [
            {"subject": f"secret-subject-{index}", "from": f"sender-{index}@example.com",
             "snippet": f"private-snippet-{index}", "unread": True, "important": False,
             "thread_id": f"thread-{index}", "source": {
                 "account_id": ACCOUNT_ID,
                 "provider_object_id": f"message-{index}",
                 "fetched_at_ms": index,
                 "source_timestamp_ms": index,
                 "etag_or_history": None,
             }}
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
    assert len(result["evidence"]["result_sha256"]) == 64
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


def test_wire_schema_rejects_wrong_types_missing_or_extra_fields():
    valid = json.loads(page(1))
    mutations = []

    wrong_bool = json.loads(page(1))
    wrong_bool["messages"][0]["unread"] = 1
    mutations.append(wrong_bool)

    missing_message_field = json.loads(page(1))
    del missing_message_field["messages"][0]["snippet"]
    mutations.append(missing_message_field)

    extra_raw_field = json.loads(page(1))
    extra_raw_field["messages"][0]["raw_payload"] = "must-not-leak"
    mutations.append(extra_raw_field)

    invalid_page_token = json.loads(page(1))
    invalid_page_token["next_page_token"] = 42
    mutations.append(invalid_page_token)

    invalid_account_id = json.loads(page(1))
    invalid_account_id["account_id"] = ACCOUNT
    mutations.append(invalid_account_id)

    for wire_value in mutations:
        result = scenarios._safe_gmail_evidence(
            gmail_events(json.dumps(wire_value)), "汇总。", ACCOUNT, True
        )
        assert result["status"] == "FAIL"
        assert result["authorization_state"] == "degraded"
        assert result["evidence"]["result_sha256"] is None
        assert "must-not-leak" not in serialized(result)

    accepted = scenarios._safe_gmail_evidence(
        gmail_events(json.dumps(valid)), "汇总。", ACCOUNT, True
    )
    assert accepted["status"] == "PASS"


def test_wire_schema_rejects_source_account_mismatch_and_invalid_source_types():
    mismatch = json.loads(page(1))
    mismatch["messages"][0]["source"]["account_id"] = "dc86763e-55f3-43de-a844-5ea32592bf35"
    bad_timestamp = json.loads(page(1))
    bad_timestamp["messages"][0]["source"]["fetched_at_ms"] = True
    oversized_subject = json.loads(page(1))
    oversized_subject["messages"][0]["subject"] = "x" * (8 * 1024 + 1)

    for wire_value in (mismatch, bad_timestamp, oversized_subject):
        result = scenarios._safe_gmail_evidence(
            gmail_events(json.dumps(wire_value)), "汇总。", ACCOUNT, True
        )
        assert result["status"] == "FAIL"
        assert result["evidence"]["result_sha256"] is None


def test_wire_hash_is_canonical_and_wrapper_raw_payload_is_rejected():
    wire_value = json.loads(page(1))
    compact = json.dumps(wire_value, separators=(",", ":"))
    reordered = json.dumps({
        "next_page_token": wire_value["next_page_token"],
        "messages": wire_value["messages"],
        "account_id": wire_value["account_id"],
    }, indent=2)
    first = scenarios._safe_gmail_evidence(gmail_events(compact), "汇总。", ACCOUNT, True)
    second = scenarios._safe_gmail_evidence(gmail_events(reordered), "汇总。", ACCOUNT, True)
    assert first["status"] == second["status"] == "PASS"
    assert first["evidence"]["result_sha256"] == second["evidence"]["result_sha256"]

    wrapped = json.dumps({"ok": True, "result": wire_value, "raw_payload": "secret"})
    rejected = scenarios._safe_gmail_evidence(
        gmail_events(wrapped), "汇总。", ACCOUNT, True
    )
    assert rejected["status"] == "FAIL"
    assert rejected["evidence"]["metadata_only"] is False
    assert "secret" not in serialized(rejected)


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
            "assertions": [
                {"name": "authorized", "passed": True},
                {"name": "durable_event_evidence", "passed": True},
            ],
            "evidence": {
                "schema_version": 1,
                "account_binding": "configured_test_account",
                "configured_account_bound": True,
                "item_count": 0,
                "item_limit": 10,
                "search_call_count": 1,
                "result_count": 1,
                "summary_bytes": 8,
                "summary_limit_bytes": 16 * 1024,
                "summary_sha256": "a" * 64,
                "result_sha256": "b" * 64,
                "event_count": 2,
                "event_size_bytes": 256,
                "event_sha256": "c" * 64,
                "metadata_only": True,
            },
            "failure": None,
        }

    monkeypatch.setattr(gmail_analysis.base, "gmail_read", fake_read)
    result = asyncio.run(gmail_analysis.run(str(tmp_path), timeout=7))
    assert observed["account"] == ACCOUNT
    assert result["status"] == "PASS"
    assert ACCOUNT not in serialized(result)


def test_production_scenario_whitelists_report_evidence(monkeypatch, tmp_path):
    monkeypatch.setenv("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", ACCOUNT)

    async def fake_read(_source_root, _account, _timeout):
        return {
            "status": "FAIL",
            "authorization_state": "degraded",
            "assertions": [{"name": "attacker-secret", "passed": True, "raw": "secret"}],
            "evidence": {"raw_payload": "private-tool-output", "messages": ["secret"]},
            "failure": "private-provider-error-with-payload",
        }

    monkeypatch.setattr(gmail_analysis.base, "gmail_read", fake_read)
    result = asyncio.run(gmail_analysis.run(str(tmp_path)))
    report = serialized(result)
    assert result["status"] == "FAIL"
    assert result["authorization_state"] == "degraded"
    assert result["failure"] == "gmail_provider_degraded"
    for secret in ("attacker-secret", "private-tool-output", "messages", "private-provider-error"):
        assert secret not in report
