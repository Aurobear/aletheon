"""Strict TOML configuration loader for reviewed Nightwatch campaigns."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

import tomllib

from .model import (
    CaseSpec,
    DiagnosticSettings,
    LabSettings,
    SourceSettings,
    require_string_sequence,
)


def _mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise TypeError(f"{field_name} must be a table")
    return value


def _integer_sequence(value: object, field_name: str) -> tuple[int, ...]:
    if not isinstance(value, list) or any(type(item) is not int for item in value):
        raise ValueError(f"{field_name} must be an array of integers")
    return tuple(value)


def _boolean(value: object, field_name: str, default: bool) -> bool:
    if value is None:
        return default
    if type(value) is not bool:
        raise TypeError(f"{field_name} must be a boolean")
    return value


def _integer(value: object, field_name: str, default: int) -> int:
    if value is None:
        return default
    if type(value) is not int:
        raise TypeError(f"{field_name} must be an integer")
    return value


def _number(value: object, field_name: str, default: float) -> float:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{field_name} must be numeric")
    return float(value)


def load_settings(path: Path) -> LabSettings:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise ValueError("nightwatch config schema_version must be 1")
    state = _mapping(data.get("state"), "state")
    source = _mapping(data.get("source"), "source")
    scheduler = _mapping(data.get("scheduler", {}), "scheduler")
    diagnostics = _mapping(data.get("diagnostics", {}), "diagnostics")
    raw_cases = data.get("campaigns")
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError("campaigns must be a non-empty table array")

    state_root = state.get("root")
    if not isinstance(state_root, str) or not state_root.strip():
        raise ValueError("state.root must be a non-empty path")
    repository_url = source.get("repository_url")
    if not isinstance(repository_url, str) or not repository_url.strip():
        raise ValueError("source.repository_url must be a non-empty string")

    cases = []
    for index, raw in enumerate(raw_cases):
        case = _mapping(raw, f"campaigns[{index}]")
        environment = _mapping(
            case.get("environment", {}), f"campaigns[{index}].environment"
        )
        if any(
            not isinstance(key, str) or not isinstance(value, str)
            for key, value in environment.items()
        ):
            raise ValueError(f"campaigns[{index}].environment must contain strings")
        cases.append(
            CaseSpec(
                case_id=str(case.get("case_id", "")),
                command=require_string_sequence(case.get("command"), "command"),
                interval_seconds=_integer(
                    case.get("interval_seconds"), "interval_seconds", 900
                ),
                timeout_seconds=_number(
                    case.get("timeout_seconds"), "timeout_seconds", 300
                ),
                expected_exit_codes=_integer_sequence(
                    case.get("expected_exit_codes", [0]), "expected_exit_codes"
                ),
                enabled=_boolean(case.get("enabled"), "enabled", True),
                diagnose_on_failure=_boolean(
                    case.get("diagnose_on_failure"), "diagnose_on_failure", True
                ),
                require_clean=_boolean(
                    case.get("require_clean"), "require_clean", True
                ),
                environment=dict(environment),
            )
        )

    return LabSettings(
        state_root=Path(state_root).expanduser().resolve(),
        source=SourceSettings(
            repository_url=repository_url,
            ref=str(source.get("ref", "dev")),
        ),
        cases=tuple(cases),
        diagnostics=DiagnosticSettings(
            enabled=_boolean(diagnostics.get("enabled"), "diagnostics.enabled", False),
            base_url=str(diagnostics.get("base_url", "https://aiapi.lejurobot.com/v1")),
            model=str(diagnostics.get("model", "deepseek/deepseek-v4-flash")),
            api_key_env=str(diagnostics.get("api_key_env", "LEJU_API_KEY")),
            timeout_seconds=_number(
                diagnostics.get("timeout_seconds"), "diagnostics.timeout_seconds", 60
            ),
            max_evidence_bytes=_integer(
                diagnostics.get("max_evidence_bytes"),
                "diagnostics.max_evidence_bytes",
                48 * 1024,
            ),
            max_output_tokens=_integer(
                diagnostics.get("max_output_tokens"),
                "diagnostics.max_output_tokens",
                2048,
            ),
            max_calls_per_day=_integer(
                diagnostics.get("max_calls_per_day"),
                "diagnostics.max_calls_per_day",
                24,
            ),
        ),
        poll_seconds=_integer(
            scheduler.get("poll_seconds"), "scheduler.poll_seconds", 60
        ),
        max_capture_bytes=_integer(
            state.get("max_capture_bytes"), "state.max_capture_bytes", 64 * 1024
        ),
    )


def expand_argument(value: str, *, repo: Path, artifacts: Path, run_id: str) -> str:
    """Expand only documented host-owned placeholders in one argv element."""
    return (
        value.replace("{repo}", str(repo))
        .replace("{artifacts}", str(artifacts))
        .replace("{run_id}", run_id)
    )
