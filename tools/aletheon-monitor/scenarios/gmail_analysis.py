"""Opt-in real Gmail scenario; credentials are never substituted with fixtures."""
from __future__ import annotations
import os
from pathlib import Path
from src import scenarios as base


async def run(source_root: str, timeout: float = 120.0) -> dict:
    account = os.environ.get("ALETHEON_PRODUCTION_GMAIL_ACCOUNT", "").strip()
    if not account:
        return {"scenario": "gmail_analysis", "status": "BLOCKED",
                "failure": "ALETHEON_PRODUCTION_GMAIL_ACCOUNT is required for the opt-in live lane",
                "assertions": [{"name": "live_test_account_configured", "passed": False}]}
    completed = await base.gmail_read(str(Path(source_root).resolve()), timeout)
    evidence = completed.get("evidence", {})
    frame = evidence.get("frame", "")
    assertions = list(completed.get("assertions", [])) + [
        {"name": "bounded_summary", "passed": len(frame.encode("utf-8")) <= 64 * 1024},
        {"name": "no_raw_message_payload", "passed": "raw_payload" not in frame and "rawBody" not in frame},
        {"name": "account_not_echoed", "passed": account not in frame},
    ]
    status = "PASS" if all(a["passed"] for a in assertions) else "FAIL"
    return {"scenario": "gmail_analysis", "status": status, "assertions": assertions,
            "evidence": {"event_count": evidence.get("events", 0), "summary_bytes": len(frame.encode("utf-8"))},
            "failure": completed.get("failure")}
