#!/usr/bin/env python3
"""Fail-closed integrity and evidence replay for coding receipts."""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys

try:
    from receipt import verify_receipt
except ImportError:
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
    from receipt import verify_receipt


def _digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _verify_v1(receipt: dict) -> tuple[bool, str]:
    expected = receipt.pop("integrity_sha256", "")
    actual = _digest(
        json.dumps(receipt, sort_keys=True, separators=(",", ":")).encode()
    )
    if expected != actual:
        return False, "receipt integrity mismatch"
    operation_id = receipt.get("operation_id", "")
    evidence = receipt.get("evidence", [])
    if (
        not operation_id
        or not evidence
        or any(item.get("operation_id") != operation_id for item in evidence)
    ):
        return False, "operation evidence mismatch"
    if not receipt.get("workspace_diff", "").strip():
        return False, "workspace diff missing"
    if not any(
        item.get("kind") == "acceptance_command" and item.get("exit_code") == 0
        for item in evidence
    ):
        return False, "successful command evidence missing"
    acceptance = receipt.get("acceptance", [])
    if not acceptance or any(
        item.get("exit_code") != 0 or item.get("timed_out")
        for item in acceptance
    ):
        return False, "acceptance failed"
    if (
        not receipt.get("verification", {}).get("passed")
        or receipt.get("terminal_status") != "verified"
    ):
        return False, "false success"
    return True, "verified"


def verify(path: str | pathlib.Path) -> tuple[bool, str]:
    try:
        value = json.loads(pathlib.Path(path).read_text())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return False, f"invalid receipt: {error}"
    if not isinstance(value, dict):
        return False, "receipt must be an object"
    version = value.get("schema_version", 1)
    if version == 1:
        return _verify_v1(value)
    if version == 2:
        return verify_receipt(value)
    return False, "unsupported receipt schema version"


if __name__ == "__main__":
    ok, message = verify(sys.argv[1])
    print(message)
    raise SystemExit(0 if ok else 1)
