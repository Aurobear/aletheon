"""Versioned Nightwatch configuration and result contracts."""

from __future__ import annotations

import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

OUTCOMES = frozenset(
    {"passed", "product_failed", "infra_blocked", "flaky", "cancelled", "invalid"}
)
FAILURE_CLASSES = frozenset(
    {
        "none",
        "assertion_failure",
        "command_failure",
        "timeout",
        "spawn_failure",
        "cleanup_failure",
        "evidence_failure",
        "infrastructure_failure",
        "configuration_failure",
    }
)


def require_identifier(value: str, field_name: str) -> str:
    """Validate a stable identifier that is safe in paths and database keys."""
    if not value or len(value) > 160:
        raise ValueError(f"{field_name} must contain 1..160 characters")
    allowed = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-")
    if any(character not in allowed for character in value):
        raise ValueError(f"{field_name} contains unsupported characters")
    if value in {".", ".."}:
        raise ValueError(f"{field_name} cannot be a relative path marker")
    return value


@dataclass(frozen=True)
class CaseSpec:
    """One reviewed command campaign.

    Commands are argv arrays from repository-owned configuration.  Natural
    language model output is never converted into a command by this layer.
    """

    case_id: str
    command: tuple[str, ...]
    interval_seconds: int = 900
    timeout_seconds: float = 300.0
    expected_exit_codes: tuple[int, ...] = (0,)
    enabled: bool = True
    diagnose_on_failure: bool = True
    require_clean: bool = True
    environment: Mapping[str, str] = field(default_factory=dict)

    def __post_init__(self) -> None:
        require_identifier(self.case_id, "case_id")
        if not self.command or any(
            not isinstance(value, str) or not value for value in self.command
        ):
            raise ValueError("command must be a non-empty argv array")
        if self.interval_seconds < 1:
            raise ValueError("interval_seconds must be positive")
        if self.timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be positive")
        if not self.expected_exit_codes:
            raise ValueError("expected_exit_codes cannot be empty")


@dataclass(frozen=True)
class DiagnosticSettings:
    enabled: bool = False
    base_url: str = "https://aiapi.lejurobot.com/v1"
    model: str = "deepseek/deepseek-v4-flash"
    api_key_env: str = "LEJU_API_KEY"
    timeout_seconds: float = 60.0
    max_evidence_bytes: int = 48 * 1024
    max_output_tokens: int = 2048
    max_calls_per_day: int = 24

    def __post_init__(self) -> None:
        if self.enabled and not self.base_url.startswith("https://"):
            raise ValueError("diagnostic base_url must use https")
        if not self.model:
            raise ValueError("diagnostic model cannot be empty")
        if not self.api_key_env or "=" in self.api_key_env:
            raise ValueError("api_key_env must be an environment variable name")
        if self.timeout_seconds <= 0:
            raise ValueError("diagnostic timeout_seconds must be positive")
        if not 1024 <= self.max_evidence_bytes <= 1024 * 1024:
            raise ValueError("max_evidence_bytes must be between 1 KiB and 1 MiB")
        if not 1 <= self.max_output_tokens <= 32_768:
            raise ValueError("max_output_tokens must be between 1 and 32768")
        if not 1 <= self.max_calls_per_day <= 10_000:
            raise ValueError("max_calls_per_day must be between 1 and 10000")


@dataclass(frozen=True)
class SourceSettings:
    repository_url: str
    ref: str = "dev"

    def __post_init__(self) -> None:
        parsed = urlsplit(self.repository_url)
        if parsed.scheme not in {"https", "file"}:
            raise ValueError("source repository_url must use https:// or file://")
        if parsed.query or parsed.fragment or parsed.username or parsed.password:
            raise ValueError(
                "source repository_url cannot contain credentials or query data"
            )
        if parsed.scheme == "https" and not parsed.hostname:
            raise ValueError("source repository_url must have a hostname")
        if parsed.scheme == "file" and (
            parsed.netloc not in {"", "localhost"} or not parsed.path.startswith("/")
        ):
            raise ValueError("file source repository_url must be local and absolute")
        if (
            not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", self.ref)
            or ".." in self.ref
            or self.ref.endswith(("/", "."))
            or self.ref.endswith(".lock")
        ):
            raise ValueError("source ref is invalid")


@dataclass(frozen=True)
class LabSettings:
    state_root: Path
    source: SourceSettings
    cases: tuple[CaseSpec, ...]
    diagnostics: DiagnosticSettings = DiagnosticSettings()
    poll_seconds: int = 60
    max_capture_bytes: int = 64 * 1024

    def __post_init__(self) -> None:
        if not self.state_root.is_absolute():
            raise ValueError("state_root must be absolute")
        if self.state_root == Path("/"):
            raise ValueError("state_root cannot be the filesystem root")
        if self.poll_seconds < 1:
            raise ValueError("poll_seconds must be positive")
        if not 1024 <= self.max_capture_bytes <= 4 * 1024 * 1024:
            raise ValueError("max_capture_bytes must be between 1 KiB and 4 MiB")
        case_ids = [case.case_id for case in self.cases]
        if len(case_ids) != len(set(case_ids)):
            raise ValueError("case_id values must be unique")


def require_string_sequence(value: object, field_name: str) -> tuple[str, ...]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes)):
        raise TypeError(f"{field_name} must be an array of strings")
    result = tuple(value)
    if any(not isinstance(item, str) for item in result):
        raise ValueError(f"{field_name} must be an array of strings")
    return result
