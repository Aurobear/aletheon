"""Opt-in real Gmail scenario; credentials are never substituted with fixtures."""
from __future__ import annotations
import os
from pathlib import Path
from src import scenarios as base


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
    evidence = completed.get("evidence", {})
    assertions = list(completed.get("assertions", [])) + [
        {"name": "live_test_account_configured", "passed": True},
        {"name": "account_binding_attested",
         "passed": evidence.get("account_binding") == "configured_test_account"
         and evidence.get("configured_account_bound") is True},
    ]
    state = completed.get("authorization_state", "degraded")
    status = "PASS" if state == "authorized" and all(item["passed"] for item in assertions) else "FAIL"
    return {"scenario": "gmail_analysis", "status": status,
            "authorization_state": state, "assertions": assertions,
            "evidence": evidence, "failure": completed.get("failure")}
