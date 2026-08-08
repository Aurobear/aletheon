"""Bounded evidence redaction before external model diagnostics."""

from __future__ import annotations

import re
from collections.abc import Iterable

_SECRET_PATTERNS = (
    re.compile(
        r"(?i)\b(?:api[_-]?key|authorization|bearer|token|secret|password)\b\s*[:=]\s*[^\s,;]+"
    ),
    re.compile(
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        re.DOTALL,
    ),
    re.compile(r"\bgh[opsu]_[A-Za-z0-9_]{20,}\b"),
    re.compile(r"\bsk-[A-Za-z0-9_-]{16,}\b"),
)


def redact(text: str, *, known_secrets: Iterable[str] = ()) -> tuple[str, int]:
    value = text
    replacements = 0
    for secret in sorted(
        {item for item in known_secrets if len(item) >= 8}, key=len, reverse=True
    ):
        count = value.count(secret)
        if count:
            value = value.replace(secret, "<redacted-secret>")
            replacements += count
    for pattern in _SECRET_PATTERNS:
        value, count = pattern.subn("<redacted-secret>", value)
        replacements += count
    return value, replacements
