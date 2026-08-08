"""Acceptance contract validation helpers."""

from datetime import datetime, timezone
import hashlib
import json
from typing import Any

ALLOWED_STATUSES = ("not_run", "infra_blocked", "failed", "passed", "waived")
ENVIRONMENT_LEVELS = ("source", "installed", "simulation", "hil", "physical_real")
_PROVENANCE_KEYS = {
    "repo", "build", "environment_level", "installed_artifact",
    "client", "daemons", "provider", "fixture_digest", "generation_id",
}

class ContractError(ValueError):
    """Raised when a contract is invalid."""

def _exact_keys(obj: Any, allowed: set[str], path: str) -> None:
    if not isinstance(obj, dict):
        raise ContractError(f"{path} must be a dict")
    unknown = set(obj) - allowed
    missing = allowed - set(obj)
    if unknown or missing:
        raise ContractError(f"{path} must have exact keys {sorted(allowed)}")

def _text(value: Any, path: str) -> str:
    if not isinstance(value, str):
        raise ContractError(f"{path} must be a string")
    if not value:
        raise ContractError(f"{path} must not be empty")
    try:
        byte_len = len(value.encode("utf-8"))
    except UnicodeEncodeError:
        raise ContractError(f"{path} must be valid Unicode text") from None
    if byte_len > 4096:
        raise ContractError(f"{path} must be at most 4096 UTF-8 bytes")
    return value

def _sha(value: Any, path: str, lengths: tuple[int, ...] = (64,)) -> str:
    if not isinstance(value, str) or len(value) not in lengths:
        raise ContractError(f"{path} must be hex string of length {' or '.join(map(str, lengths))}")
    if not all(c in "0123456789abcdef" for c in value):
        raise ContractError(f"{path} must be lowercase hex")
    return value

def _features(value: Any, path: str) -> list[str]:
    if not isinstance(value, list) or not all(isinstance(x, str) for x in value):
        raise ContractError(f"{path} must be a list of strings")
    result: list[str] = []
    seen: set[str] = set()
    for item in value:
        item = _text(item, path)
        if item in seen:
            raise ContractError(f"{path} contains duplicate feature {item!r}")
        if result and item < result[-1]:
            raise ContractError(f"{path} features must be sorted")
        seen.add(item)
        result.append(item)
    return result

def _repo(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"sha", "dirty"}, "repo")
    dirty = value["dirty"]
    if not isinstance(dirty, bool):
        raise ContractError("repo.dirty must be a bool")
    return {"sha": _sha(value["sha"], "repo.sha", (40, 64)), "dirty": dirty}

def _build(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"profile", "features"}, "build")
    return {
        "profile": _text(value["profile"], "build.profile"),
        "features": _features(value["features"], "build.features"),
    }

def _installed_artifact(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"path", "sha256"}, "installed_artifact")
    return {
        "path": _text(value["path"], "installed_artifact.path"),
        "sha256": _sha(value["sha256"], "installed_artifact.sha256"),
    }

def _client(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"version", "protocol_version"}, "client")
    return {
        "version": _text(value["version"], "client.version"),
        "protocol_version": _text(value["protocol_version"], "client.protocol_version"),
    }

def _daemon(value: Any, path: str) -> dict[str, Any]:
    _exact_keys(value, {"path", "sha256", "version", "protocol_version"}, path)
    return {
        "path": _text(value["path"], path + ".path"),
        "sha256": _sha(value["sha256"], path + ".sha256"),
        "version": _text(value["version"], path + ".version"),
        "protocol_version": _text(value["protocol_version"], path + ".protocol_version"),
    }

def _daemons(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"machine", "user"}, "daemons")
    return {
        "machine": _daemon(value["machine"], "daemons.machine"),
        "user": _daemon(value["user"], "daemons.user"),
    }

def _provider(value: Any) -> dict[str, Any]:
    _exact_keys(value, {"provider_id", "model_id", "endpoint_id"}, "provider")
    endpoint = _text(value["endpoint_id"], "provider.endpoint_id")
    lowered = endpoint.lower()
    if any(token in lowered for token in ("://", "@", "?", "#", "key", "token", "secret", "password")):
        raise ContractError("provider.endpoint_id contains forbidden substring")
    return {
        "provider_id": _text(value["provider_id"], "provider.provider_id"),
        "model_id": _text(value["model_id"], "provider.model_id"),
        "endpoint_id": endpoint,
    }

def validate_provenance(value: dict) -> dict:
    """Validate and normalize a provenance record."""
    _exact_keys(value, _PROVENANCE_KEYS, "provenance")
    repo = _repo(value["repo"])
    build = _build(value["build"])
    env_level = _text(value["environment_level"], "environment_level")
    if env_level not in ENVIRONMENT_LEVELS:
        raise ContractError("environment_level must be one of " + ", ".join(ENVIRONMENT_LEVELS))
    installed_artifact = _installed_artifact(value["installed_artifact"])
    client = _client(value["client"])
    daemons = _daemons(value["daemons"])
    if env_level == "installed":
        if installed_artifact["path"] != "/usr/bin/aletheon":
            raise ContractError("installed environment requires installed_artifact.path /usr/bin/aletheon")
        for daemon_key in ("machine", "user"):
            daemon = daemons[daemon_key]
            if daemon["path"] != "/usr/bin/aletheon":
                raise ContractError(f"installed environment requires daemons.{daemon_key}.path /usr/bin/aletheon")
            if daemon["sha256"] != installed_artifact["sha256"]:
                raise ContractError(f"daemons.{daemon_key}.sha256 must match installed_artifact.sha256 in installed environment")
            if daemon["version"] != client["version"]:
                raise ContractError(f"daemons.{daemon_key}.version must equal client.version in installed environment")
            if daemon["protocol_version"] != client["protocol_version"]:
                raise ContractError(f"daemons.{daemon_key}.protocol_version must equal client.protocol_version in installed environment")
    generation_id = _text(value["generation_id"], "generation_id")
    if not all(("a" <= c <= "z") or ("A" <= c <= "Z") or ("0" <= c <= "9") or c in "._-" for c in generation_id):
        raise ContractError("generation_id must use ASCII letters, digits, dot, underscore, hyphen")
    return {
        "repo": repo,
        "build": build,
        "environment_level": env_level,
        "installed_artifact": installed_artifact,
        "client": client,
        "daemons": daemons,
        "provider": _provider(value["provider"]),
        "fixture_digest": _sha(value["fixture_digest"], "fixture_digest"),
        "generation_id": generation_id,
    }

def validate_waiver(value: dict, p0: bool = False, reference_time: datetime | None = None) -> dict:
    """Validate a waiver record; p0 waivers and expired waivers are rejected.

    Args:
        value: The waiver dict with approver, reason, expires_at.
        p0: If True, P0 waivers are rejected outright.
        reference_time: If provided, the waiver must not have expired at this time.
                        Defaults to datetime.now(timezone.utc).
    """
    if p0:
        raise ContractError("p0 waivers are not accepted")
    _exact_keys(value, {"approver", "reason", "expires_at"}, "waiver")
    expires_at = _text(value["expires_at"], "expires_at")
    if len(expires_at) != 20:
        raise ContractError("expires_at must be YYYY-MM-DDTHH:MM:SSZ")
    try:
        expiry_dt = datetime.strptime(expires_at, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        raise ContractError("expires_at must be YYYY-MM-DDTHH:MM:SSZ") from None
    if reference_time is None:
        reference_time = datetime.now(timezone.utc)
    if expiry_dt <= reference_time:
        raise ContractError(f"waiver expired at {expires_at} (reference {reference_time.strftime('%Y-%m-%dT%H:%M:%SZ')})")
    return {
        "approver": _text(value["approver"], "approver"),
        "reason": _text(value["reason"], "reason"),
        "expires_at": expires_at,
    }

def canonical_bytes(value: Any) -> bytes:
    """Return compact sorted UTF-8 JSON bytes plus newline."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8") + b"\n"

def sha256_json(value: Any) -> str:
    """Return SHA-256 hex digest of canonical_bytes(value)."""
    return hashlib.sha256(canonical_bytes(value)).hexdigest()
