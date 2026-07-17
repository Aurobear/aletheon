"""Opt-in real Gmail scenario; credentials are never substituted with fixtures."""
from __future__ import annotations
import os
from pathlib import Path
from src import scenarios as base


_ASSERTION_NAMES = {
    "turn_done", "single_bounded_search", "configured_account_bound", "authorized",
    "result_schema_bounded", "metadata_only", "summary_bounded_and_redacted",
    "durable_event_evidence",
}
_FAILURES = {
    None,
    "google_unauthorized_account", "google_scope_denied",
    "google_credential_unavailable", "google_reauthorization_required",
    "gmail_evidence_incomplete", "gmail_provider_degraded", "gmail_turn_incomplete",
    "gmail_summary_unsafe",
}


def _digest(value: object) -> str | None:
    if not isinstance(value, str) or len(value) != 64:
        return None
    return value if all(character in "0123456789abcdef" for character in value) else None


def _redacted_evidence(value: object) -> dict:
    source = value if isinstance(value, dict) else {}
    integer_names = (
        "search_call_count", "result_count", "item_count", "item_limit",
        "summary_bytes", "summary_limit_bytes", "event_count", "event_size_bytes",
    )
    evidence = {
        "schema_version": source.get("schema_version") if source.get("schema_version") == 1 else None,
        "account_binding": (
            "configured_test_account"
            if source.get("account_binding") == "configured_test_account" else None
        ),
        "configured_account_bound": source.get("configured_account_bound") is True,
        "metadata_only": source.get("metadata_only") is True,
        "summary_sha256": _digest(source.get("summary_sha256")),
        "result_sha256": _digest(source.get("result_sha256")),
        "event_sha256": _digest(source.get("event_sha256")),
    }
    for name in integer_names:
        item = source.get(name)
        evidence[name] = item if isinstance(item, int) and not isinstance(item, bool) and item >= 0 else None
    return evidence


async def run(source_root: str, timeout: float = 120.0) -> dict:
    account = os.environ.get("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", "").strip()
    if not account:
        return {"scenario": "gmail_analysis", "status": "BLOCKED",
                "authorization_state": "degraded",
                "failure": "gmail_test_account_not_configured",
                "assertions": [{"name": "live_test_account_configured", "passed": False}]}
    if len(account.encode("utf-8")) > 128 or any(ord(character) < 32 for character in account):
        return {"scenario": "gmail_analysis", "status": "FAIL",
                "authorization_state": "degraded",
                "failure": "gmail_test_account_reference_invalid",
                "assertions": [{"name": "live_test_account_configured", "passed": False}]}
    completed = await base.gmail_read(str(Path(source_root).resolve()), account, timeout)
    evidence = _redacted_evidence(completed.get("evidence"))
    assertions = [
        {"name": item.get("name"), "passed": item.get("passed") is True}
        for item in completed.get("assertions", [])
        if isinstance(item, dict) and item.get("name") in _ASSERTION_NAMES
    ] + [
        {"name": "live_test_account_configured", "passed": True},
        {"name": "account_binding_attested",
         "passed": evidence.get("account_binding") == "configured_test_account"
         and evidence.get("configured_account_bound") is True},
    ]
    state = completed.get("authorization_state", "degraded")
    if state not in {"authorized", "unauthorized", "degraded"}:
        state = "degraded"
    wire_attested = (
        evidence.get("schema_version") == 1
        and evidence.get("item_limit") == 10
        and isinstance(evidence.get("item_count"), int)
        and evidence["item_count"] <= evidence["item_limit"]
        and evidence.get("result_sha256") is not None
        and evidence.get("summary_sha256") is not None
        and evidence.get("event_sha256") is not None
        and isinstance(evidence.get("event_count"), int)
        and isinstance(evidence.get("event_size_bytes"), int)
        and evidence.get("metadata_only") is True
    )
    assertions.append({"name": "wire_schema_attested", "passed": wire_attested})
    status = "PASS" if state == "authorized" and all(item["passed"] for item in assertions) else "FAIL"
    failure = completed.get("failure")
    if failure not in _FAILURES:
        failure = "gmail_provider_degraded"
    return {"scenario": "gmail_analysis", "status": status,
            "authorization_state": state, "assertions": assertions,
            "evidence": evidence, "failure": failure}
