"""Append-only supervisor evidence and atomically sealed run bundles."""

from __future__ import annotations

import hashlib
import json
import os
import tempfile
from collections.abc import Mapping
from datetime import datetime, timezone
from pathlib import Path

from .model import require_identifier


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def canonical_json(value: object) -> bytes:
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        + "\n"
    ).encode()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def atomic_write(path: Path, data: bytes, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            os.close(descriptor)
        except OSError:
            pass
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


class EvidenceBundle:
    """One run directory owned by the external supervisor."""

    def __init__(self, artifacts_root: Path, run_id: str):
        require_identifier(run_id, "run_id")
        self.run_id = run_id
        self.path = artifacts_root / run_id
        self.path.mkdir(parents=True, exist_ok=False, mode=0o700)
        os.chmod(self.path, 0o700)
        self.events_path = self.path / "supervisor-events.jsonl"
        self._sealed = False

    def write_json(self, name: str, value: object) -> Path:
        if self._sealed:
            raise RuntimeError("bundle is already sealed")
        if Path(name).name != name or not name.endswith(".json"):
            raise ValueError("JSON evidence name must be a simple .json filename")
        target = self.path / name
        atomic_write(target, canonical_json(value))
        return target

    def append_event(
        self, event_type: str, fields: Mapping[str, object] | None = None
    ) -> None:
        if self._sealed:
            raise RuntimeError("bundle is already sealed")
        require_identifier(event_type, "event_type")
        event = {
            "schema_version": 1,
            "run_id": self.run_id,
            "recorded_at": utc_now(),
            "event_type": event_type,
            **dict(fields or {}),
        }
        descriptor = os.open(
            self.events_path,
            os.O_WRONLY | os.O_APPEND | os.O_CREAT,
            0o600,
        )
        try:
            os.write(descriptor, canonical_json(event))
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

    def seal(self) -> Path:
        if self._sealed:
            raise RuntimeError("bundle is already sealed")
        entries = []
        for path in sorted(self.path.iterdir(), key=lambda item: item.name):
            if path.is_file() and path.name != "bundle.json":
                entries.append(
                    {
                        "name": path.name,
                        "size": path.stat().st_size,
                        "sha256": sha256_file(path),
                    }
                )
        receipt = {
            "schema_version": 1,
            "run_id": self.run_id,
            "sealed_at": utc_now(),
            "files": entries,
        }
        target = self.path / "bundle.json"
        atomic_write(target, canonical_json(receipt))
        self._sealed = True
        return target
