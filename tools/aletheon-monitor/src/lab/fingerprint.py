"""Deterministic failure normalization and clustering."""

from __future__ import annotations

import hashlib
import json
import re
from collections.abc import Mapping, Sequence

_DYNAMIC_PATTERNS = (
    (
        re.compile(
            r"\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b",
            re.IGNORECASE,
        ),
        "<uuid>",
    ),
    (re.compile(r"\b(?:pid|process)[=: ]+\d+\b", re.IGNORECASE), "pid=<pid>"),
    (
        re.compile(
            r"\b(?:run|session|turn|operation)[_-]?id[=: ]+[A-Za-z0-9._:-]+",
            re.IGNORECASE,
        ),
        "id=<id>",
    ),
    (re.compile(r"\b20\d\d-\d\d-\d\d[T ][0-9:.+-]+Z?\b"), "<timestamp>"),
    (re.compile(r"/tmp/[A-Za-z0-9._/-]+"), "/tmp/<path>"),
    (re.compile(r"/var/tmp/[A-Za-z0-9._/-]+"), "/var/tmp/<path>"),
    (re.compile(r"\b0x[0-9a-f]+\b", re.IGNORECASE), "<address>"),
    (re.compile(r"\b[0-9a-f]{40,64}\b", re.IGNORECASE), "<digest>"),
)
_WHITESPACE = re.compile(r"\s+")


def normalize_signature(value: str, limit: int = 8192) -> str:
    normalized = value[:limit]
    for pattern, replacement in _DYNAMIC_PATTERNS:
        normalized = pattern.sub(replacement, normalized)
    return _WHITESPACE.sub(" ", normalized).strip().casefold()


def failure_fingerprint(
    *,
    case_id: str,
    failure_class: str,
    signature: str,
    invariant_ids: Sequence[str] = (),
    component: str = "external-supervisor",
) -> str:
    """Return a model-independent fingerprint for one failure shape."""
    payload = {
        "schema_version": 1,
        "case_id": case_id,
        "failure_class": failure_class,
        "invariant_ids": sorted(set(invariant_ids)),
        "component": component,
        "signature": normalize_signature(signature),
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def signature_from_result(result: Mapping[str, object]) -> str:
    process = result.get("process")
    if not isinstance(process, Mapping):
        return str(result.get("failure_class", "invalid"))
    stderr = process.get("stderr_preview")
    stdout = process.get("stdout_preview")
    return "\n".join(
        part for part in (str(stderr or ""), str(stdout or "")) if part
    ) or str(result.get("failure_class", "invalid"))
