#!/usr/bin/env python3
"""Strict benchmark task contracts for real coding evaluation."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path, PurePosixPath
import tomllib


TASK_KEYS = frozenset(
    {
        "schema_version",
        "id",
        "category",
        "fixture",
        "prompt",
        "timeout_secs",
        "acceptance_commands",
        "forbidden_paths",
        "required_changed_paths",
        "expected_terminal",
        "setup",
        "resource_checks",
    }
)
TERMINALS = frozenset({"verified", "blocked", "budget_exhausted", "cancelled"})
CATEGORIES = frozenset(
    {
        "behavioral_bugfix",
        "diagnosis",
        "multifile_change",
        "regression_test",
        "config_schema",
        "lint",
        "documentation",
        "dirty_workspace",
        "budget",
        "approval",
    }
)
SETUP_KEYS = frozenset({"dirty_path", "dirty_content", "exec_max_turns"})
RESOURCE_CHECKS = frozenset({"no_descendant_processes"})
MAX_TEXT_BYTES = 64 * 1024
MAX_LIST_ITEMS = 128


class ContractError(ValueError):
    """A benchmark document is malformed or unsafe."""


def _text(value: object, field: str, *, maximum: int = MAX_TEXT_BYTES) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ContractError(f"{field} must be non-empty text")
    if len(value.encode()) > maximum:
        raise ContractError(f"{field} exceeds {maximum} bytes")
    return value


def _relative_path(value: object, field: str, *, prefix: bool = False) -> str:
    text = _text(value, field, maximum=4096).replace("\\", "/")
    trailing = text.endswith("/")
    path = PurePosixPath(text.rstrip("/"))
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise ContractError(f"{field} must be a normalized relative path")
    normalized = path.as_posix()
    return normalized + "/" if prefix and trailing else normalized


def _string_list(value: object, field: str, *, paths: bool = False) -> tuple[str, ...]:
    if not isinstance(value, list) or len(value) > MAX_LIST_ITEMS:
        raise ContractError(f"{field} must be a bounded list")
    items = tuple(
        _relative_path(item, f"{field}[]", prefix=True) if paths else _text(item, f"{field}[]")
        for item in value
    )
    if len(items) != len(set(items)):
        raise ContractError(f"{field} contains duplicates")
    return items


def _commands(value: object) -> tuple[tuple[str, ...], ...]:
    if not isinstance(value, list) or not value or len(value) > MAX_LIST_ITEMS:
        raise ContractError("acceptance_commands must be a non-empty bounded list")
    commands: list[tuple[str, ...]] = []
    for raw in value:
        if not isinstance(raw, list) or not raw:
            raise ContractError("acceptance command argv must be non-empty")
        commands.append(tuple(_text(arg, "acceptance command argument", maximum=4096) for arg in raw))
    return tuple(commands)


def _setup(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ContractError("setup must be a table")
    unknown = set(value) - SETUP_KEYS
    if unknown:
        raise ContractError(f"setup contains unknown fields: {sorted(unknown)}")
    result: dict[str, object] = {}
    if "dirty_path" in value or "dirty_content" in value:
        if set(value).isdisjoint({"dirty_path", "dirty_content"}) or not {
            "dirty_path",
            "dirty_content",
        }.issubset(value):
            raise ContractError("dirty_path and dirty_content must be supplied together")
        result["dirty_path"] = _relative_path(value["dirty_path"], "setup.dirty_path")
        result["dirty_content"] = _text(value["dirty_content"], "setup.dirty_content")
    if "exec_max_turns" in value:
        turns = value["exec_max_turns"]
        if not isinstance(turns, int) or isinstance(turns, bool) or not 1 <= turns <= 1000:
            raise ContractError("setup.exec_max_turns must be in 1..=1000")
        result["exec_max_turns"] = turns
    return result


@dataclass(frozen=True)
class BenchmarkTask:
    schema_version: int
    id: str
    category: str
    fixture: str
    prompt: str
    timeout_secs: int
    acceptance_commands: tuple[tuple[str, ...], ...]
    forbidden_paths: tuple[str, ...]
    required_changed_paths: tuple[str, ...]
    expected_terminal: str
    setup: dict[str, object]
    resource_checks: tuple[str, ...]
    source: Path

    @classmethod
    def from_mapping(cls, value: object, source: Path) -> "BenchmarkTask":
        if not isinstance(value, dict):
            raise ContractError("task document must be a table")
        keys = set(value)
        if keys != TASK_KEYS:
            raise ContractError(
                f"task fields differ: missing={sorted(TASK_KEYS - keys)} "
                f"unknown={sorted(keys - TASK_KEYS)}"
            )
        schema_version = value["schema_version"]
        if schema_version != 1:
            raise ContractError("schema_version must be 1")
        task_id = _text(value["id"], "id", maximum=128)
        if not all(character.islower() or character.isdigit() or character == "_" for character in task_id):
            raise ContractError("id must use lowercase letters, digits, and underscores")
        category = _text(value["category"], "category", maximum=128)
        if category not in CATEGORIES:
            raise ContractError(f"unsupported category: {category}")
        fixture = _relative_path(value["fixture"], "fixture")
        timeout_secs = value["timeout_secs"]
        if not isinstance(timeout_secs, int) or isinstance(timeout_secs, bool) or not 1 <= timeout_secs <= 3600:
            raise ContractError("timeout_secs must be in 1..=3600")
        forbidden = _string_list(value["forbidden_paths"], "forbidden_paths", paths=True)
        required = _string_list(value["required_changed_paths"], "required_changed_paths", paths=True)
        if any(
            left == right or left.startswith(right.rstrip("/") + "/") or right.startswith(left.rstrip("/") + "/")
            for left in forbidden
            for right in required
        ):
            raise ContractError("required and forbidden paths overlap")
        expected = _text(value["expected_terminal"], "expected_terminal", maximum=64)
        if expected not in TERMINALS:
            raise ContractError(f"unsupported expected_terminal: {expected}")
        resources = _string_list(value["resource_checks"], "resource_checks")
        unknown_resources = set(resources) - RESOURCE_CHECKS
        if unknown_resources:
            raise ContractError(f"unsupported resource checks: {sorted(unknown_resources)}")
        return cls(
            schema_version=schema_version,
            id=task_id,
            category=category,
            fixture=fixture,
            prompt=_text(value["prompt"], "prompt"),
            timeout_secs=timeout_secs,
            acceptance_commands=_commands(value["acceptance_commands"]),
            forbidden_paths=forbidden,
            required_changed_paths=required,
            expected_terminal=expected,
            setup=_setup(value["setup"]),
            resource_checks=resources,
            source=source,
        )


def load_task(path: Path, root: Path) -> BenchmarkTask:
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ContractError(f"cannot read task {path}: {error}") from error
    task = BenchmarkTask.from_mapping(raw, path)
    fixture = root / "tests/coding/fixtures" / task.fixture
    if not fixture.is_dir():
        raise ContractError(f"fixture does not exist: {task.fixture}")
    dirty_path = task.setup.get("dirty_path")
    if isinstance(dirty_path, str) and not (fixture / dirty_path).is_file():
        raise ContractError(f"dirty path does not exist in fixture: {dirty_path}")
    return task


def load_catalog(paths: list[Path], root: Path) -> list[BenchmarkTask]:
    tasks = [load_task(path, root) for path in paths]
    ids = [task.id for task in tasks]
    if len(ids) != len(set(ids)):
        raise ContractError("catalog contains duplicate task ids")
    return sorted(tasks, key=lambda task: task.id)
