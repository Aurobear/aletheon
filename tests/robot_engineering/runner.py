#!/usr/bin/env python3
"""E1 Robot Engineering Suite Runner.

Fixture-validation mode: deterministic oracle execution against baseline and
solution workspaces. Never reports model acceptance.

Real-evaluation mode: invokes the installed `/usr/bin/aletheon` binary through
the official user socket, following the proven pattern in
tests/coding/harness/run.py.

Design constraints:
- Production real_evaluate_one locks /usr/bin/aletheon.
- Tests inject a fake executable via the private _real_evaluate_one_with_binary
  function parameter. The fake records argv and emits controlled Exec terminal JSON.
- Permission mapping to aletheon --permission-mode: read_only->safe, all others->dev.
  'full' is never granted.
- Idempotency key is deterministic: derived from bounded/normalized run_id + task_id.
- Every result carries provenance: binary SHA-256, bounded stdout/stderr, full-byte
  digests, truncation flags, sanitized argv, XDG_RUNTIME_DIR socket path, process-group status.
- Early failure returns the same typed result schema.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from typing import Dict, List, Optional, Tuple

BASE = Path(__file__).resolve().parent

try:
    import resource
except ImportError:
    resource = None  # Windows / non-POSIX


# ── constants ─────────────────────────────────────────────────────
MAX_CAPTURE = 64 * 1024
# Authoritative ExecTerminalKind variants from crates/fabric/src/types/exec.rs:7-74.
# provider_rejected_request is NOT an Exec terminal status; it is a separate
# rendered error/event semantic.  provider_unavailable and provider_rejected
# are preserved because they are legitimate ExecTerminalKind variants.
EXEC_TERMINAL_STATUSES = frozenset({
    "completed",
    "blocked",
    "cancelled",
    "provider_unavailable",
    "provider_rejected",
    "validation_failed",
    "output_backpressure",
    "failed",
})
# Non-completed terminal statuses that MUST be rejected before the
# completed_normally/oracle gates, even when the process exits 0 and
# completed_normally is a lie.  "failed" is handled earlier as
# inference_error; provider_unavailable/provider_rejected are handled
# as provider_error.  Everything else that is not "completed" is
# blocked here so a forged terminal cannot sneak past.
_NON_COMPLETED_TERMINAL_STATUSES = frozenset({
    "blocked", "cancelled", "validation_failed",
    "output_backpressure",
})

# aletheon exec --permission-mode only accepts: safe, dev, full.
# We map our internal permissions to the least-privilege compatible mode.
# read_only  -> safe   (no file writes)
# device_action, code_modification, simulation -> dev
# We NEVER grant 'full'.
PERMISSION_TO_ALETHEON_MODE = {
    "read_only": "safe",
    "device_action": "dev",
    "code_modification": "dev",
    "simulation": "dev",
}

# ── catalog validation ────────────────────────────────────────────
_CATALOG_REQUIRED_KEYS = {
    "id": str, "category": str, "provenance": str, "license": str,
    "source_kind": str, "network": str, "permission": str, "risk": str,
    "timeout": int, "cpu_seconds": int, "memory_mb": int,
    "max_open_files": int, "max_processes": int,
    "injected_failures": list, "expected_failure_class": str,
    "task_prompt": str, "allowed_paths": list,
    "forbidden_paths": list, "required_paths": list,
    "oracle_path": str, "solution_path": str, "fixture_path": str,
}
_VALID_PERMISSIONS = frozenset(PERMISSION_TO_ALETHEON_MODE.keys())
_VALID_RISKS = frozenset({"low", "medium", "high"})
_VALID_SOURCE_KINDS = frozenset({"python"})
_VALID_FAILURE_CLASSES = frozenset({
    "latency", "reordering", "packet_loss", "endianness",
    "clock_drift", "device_rejection", "watchdog",
})
_RUN_ID_REGEX = r"^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$"

# Required exact category distribution per spec
_REQUIRED_CATEGORY_COUNTS = {
    "ros2_lifecycle_launch": 6,
    "containers_build": 4,
    "can_ethercat_serial": 5,
    "control_numerics": 5,
    "sensing_logs": 4,
    "safety_device_protocol": 3,
    "multi_file_engineering": 3,
}


def _validate_catalog(tasks: List[dict], base: Path) -> List[str]:
    """Fail-closed catalog validation. Returns list of error strings.

    Every task is validated independently; an error in one task does NOT
    cause later tasks to skip validation.
    """
    errors: List[str] = []
    if not isinstance(tasks, list):
        return ["catalog: tasks must be a non-empty list"]
    if len(tasks) != 30:
        errors.append(f"catalog: expected 30 tasks, found {len(tasks)}")
    if len(tasks) == 0:
        return errors if errors else ["catalog: tasks must be a non-empty list"]

    # ── exact category distribution ──
    category_counts: dict = {}
    for t in tasks:
        if not isinstance(t, dict):
            errors.append(f"catalog: task {t!r} is not a dict")
            continue
        cat = t.get("category")
        if isinstance(cat, str):
            category_counts[cat] = category_counts.get(cat, 0) + 1
    if category_counts != _REQUIRED_CATEGORY_COUNTS:
        errors.append(
            f"catalog: category distribution {dict(sorted(category_counts.items()))} "
            f"does not match required {_REQUIRED_CATEGORY_COUNTS}"
        )

    seen_ids: set = set()
    seen_oracles: dict = {}

    for t in tasks:
        if not isinstance(t, dict):
            # Already reported during category_counts; skip rest.
            continue
        # ── unknown keys (reject extras) ──
        unknown_keys = set(t.keys()) - _CATALOG_REQUIRED_KEYS.keys()
        if unknown_keys:
            errors.append(
                f"{t.get('id', '?')}: unknown keys: {sorted(unknown_keys)}"
            )

        # ── required keys and types ──
        task_has_fatal_key_error = False
        for key, typ in _CATALOG_REQUIRED_KEYS.items():
            if key not in t:
                errors.append(f"{t.get('id', '?')}: missing key {key}")
                task_has_fatal_key_error = True
                continue
            val = t[key]
            if not isinstance(val, typ):
                errors.append(
                    f"{t.get('id', '?')}: {key} expected {typ.__name__}, "
                    f"got {type(val).__name__}"
                )
                task_has_fatal_key_error = True
                continue
            # Reject bool-as-int: bool is a subclass of int in Python
            if typ is int and isinstance(val, bool):
                errors.append(
                    f"{t.get('id', '?')}: {key} is bool, expected int"
                )
                task_has_fatal_key_error = True
            # Reject bool-as-list
            if typ is list and isinstance(val, bool):
                errors.append(
                    f"{t.get('id', '?')}: {key} is bool, expected list"
                )
                task_has_fatal_key_error = True
        if task_has_fatal_key_error:
            continue

        tid = t["id"]
        if not isinstance(tid, str) or not tid:
            errors.append(f"task: invalid id {tid!r}")
            continue

        # ── normalized task ID: e1_<category>_NNNN ──
        # Strip e1_ prefix, then split last 4-digit sequence
        if not tid.startswith("e1_"):
            errors.append(f"{tid}: task id must start with 'e1_'")
        else:
            rest = tid[3:]  # remove "e1_"
            # Find last underscore before the 4-digit sequence
            # e.g. "can_ethercat_serial_0001" -> cat="can_ethercat_serial", seq="0001"
            parts = rest.rsplit("_", 1)
            if len(parts) != 2:
                errors.append(f"{tid}: task id must be e1_<category>_NNNN format")
            else:
                cat_prefix, seq_str = parts
                if cat_prefix not in _REQUIRED_CATEGORY_COUNTS:
                    errors.append(
                        f"{tid}: category prefix {cat_prefix!r} not in "
                        f"{sorted(_REQUIRED_CATEGORY_COUNTS.keys())}"
                    )
                if not (len(seq_str) == 4 and seq_str.isdigit()):
                    errors.append(
                        f"{tid}: sequence must be exactly 4 digits, got {seq_str!r}"
                    )
                elif cat_prefix in _REQUIRED_CATEGORY_COUNTS:
                    max_seq = _REQUIRED_CATEGORY_COUNTS[cat_prefix]
                    seq_num = int(seq_str)
                    if seq_num < 1 or seq_num > max_seq:
                        errors.append(
                            f"{tid}: sequence {seq_num} out of range [1, {max_seq}] "
                            f"for category {cat_prefix}"
                        )
                # also ensure category field matches prefix
                if t.get("category") != cat_prefix:
                    errors.append(
                        f"{tid}: category field {t.get('category')!r} "
                        f"does not match id prefix {cat_prefix!r}"
                    )

        # ── nonempty provenance / license / category ──
        for nk in ("provenance", "license", "category"):
            val = t.get(nk, "")
            if not isinstance(val, str) or not val.strip():
                errors.append(f"{tid}: {nk} must be a nonempty string")
        if tid in seen_ids:
            errors.append(f"{tid}: duplicate task id")
        seen_ids.add(tid)

        # ── enums ──
        if t["permission"] not in _VALID_PERMISSIONS:
            errors.append(f"{tid}: invalid permission {t['permission']!r}")
        if t["risk"] not in _VALID_RISKS:
            errors.append(f"{tid}: invalid risk {t['risk']!r}")
        if t["source_kind"] not in _VALID_SOURCE_KINDS:
            errors.append(f"{tid}: invalid source_kind {t['source_kind']!r}")
        if t["network"] != "deny-all":
            errors.append(f"{tid}: network must be deny-all")

        # ── bounded integers with reasonable upper bounds ──
        if t["timeout"] <= 0 or t["timeout"] > 3600:
            errors.append(f"{tid}: timeout must be >0 and <=3600")
        if t["cpu_seconds"] <= 0 or t["cpu_seconds"] > 60:
            errors.append(f"{tid}: cpu_seconds must be >0 and <=60")
        if t["memory_mb"] <= 0 or t["memory_mb"] > 4096:
            errors.append(f"{tid}: memory_mb must be >0 and <=4096")
        if t["max_open_files"] <= 0 or t["max_open_files"] > 1024:
            errors.append(f"{tid}: max_open_files must be >0 and <=1024")
        if t["max_processes"] <= 0 or t["max_processes"] > 8192:
            errors.append(f"{tid}: max_processes must be >0 and <=8192")

        # ── nonempty typed lists ──
        for list_key in ("injected_failures", "allowed_paths",
                          "forbidden_paths", "required_paths"):
            lst = t[list_key]
            if not isinstance(lst, list) or len(lst) == 0:
                errors.append(f"{tid}: {list_key} must be a nonempty list")
                continue
            # Every element must be a nonempty string BEFORE duplicate/set
            str_err = False
            for idx, elem in enumerate(lst):
                if not isinstance(elem, str) or not elem:
                    errors.append(
                        f"{tid}: {list_key}[{idx}] must be a nonempty string, "
                        f"got {type(elem).__name__}: {elem!r}"
                    )
                    str_err = True
            if str_err:
                continue
            # No duplicate entries (safe: all elements are nonempty strings)
            if len(lst) != len(set(lst)):
                errors.append(f"{tid}: {list_key} has duplicate entries")

        # ── failure classes ──
        for fc in t["injected_failures"]:
            if not isinstance(fc, str) or fc not in _VALID_FAILURE_CLASSES:
                errors.append(f"{tid}: invalid injected_failure {fc!r}")
        if t["expected_failure_class"] not in _VALID_FAILURE_CLASSES:
            errors.append(
                f"{tid}: invalid expected_failure_class "
                f"{t['expected_failure_class']!r}"
            )
        # expected_failure_class MUST appear in injected_failures
        if t["expected_failure_class"] not in t["injected_failures"]:
            errors.append(
                f"{tid}: expected_failure_class {t['expected_failure_class']!r} "
                f"not in injected_failures"
            )

        # ── task_prompt ──
        if not t["task_prompt"] or not isinstance(t["task_prompt"], str):
            errors.append(f"{tid}: task_prompt empty or invalid")
        if tid not in t["task_prompt"]:
            errors.append(f"{tid}: task_prompt missing task id")

        # ── exact asset-root validation ──
        expected_fixture = f"fixtures/tasks/{tid}"
        expected_oracle = f"fixtures/oracles/{tid}.py"
        expected_solution = f"fixtures/solutions/{tid}"
        if t["fixture_path"] != expected_fixture:
            errors.append(
                f"{tid}: fixture_path must be {expected_fixture}, "
                f"got {t['fixture_path']}"
            )
        if t["oracle_path"] != expected_oracle:
            errors.append(
                f"{tid}: oracle_path must be {expected_oracle}, "
                f"got {t['oracle_path']}"
            )
        if t["solution_path"] != expected_solution:
            errors.append(
                f"{tid}: solution_path must be {expected_solution}, "
                f"got {t['solution_path']}"
            )

        # ── path normalization: relative, no traversal, no backslashes ──
        for pk in ("oracle_path", "solution_path", "fixture_path"):
            p = t[pk]
            if not isinstance(p, str) or not p:
                errors.append(f"{tid}: {pk} empty")
                continue
            if p.startswith("/"):
                errors.append(f"{tid}: {pk} is absolute: {p}")
            if "\\" in p:
                errors.append(f"{tid}: {pk} has backslashes: {p}")
            if ".." in p.split("/"):
                errors.append(f"{tid}: {pk} has path traversal: {p}")

        # ── file paths: no directory separators in allowed/forbidden/required ──
        for pk in ("allowed_paths", "forbidden_paths", "required_paths"):
            for p in t[pk]:
                if not isinstance(p, str) or not p:
                    errors.append(f"{tid}: empty entry in {pk}")
                elif "/" in p or "\\" in p:
                    errors.append(
                        f"{tid}: path {p!r} in {pk} has directory separator"
                    )

        # ── no overlap between allowed and forbidden ──
        # Only perform set operations when all elements are known-safe strings
        list_keys_ok = True
        for list_key in ("allowed_paths", "forbidden_paths", "required_paths"):
            lst = t[list_key]
            if not isinstance(lst, list):
                list_keys_ok = False
                break
            for elem in lst:
                if not isinstance(elem, str) or not elem:
                    list_keys_ok = False
                    break
        if list_keys_ok:
            allowed = set(t["allowed_paths"])
            forbidden = set(t["forbidden_paths"])
            overlap = allowed & forbidden
            if overlap:
                errors.append(
                    f"{tid}: paths in both allowed and forbidden: {overlap}"
                )

            # ── required paths must be a subset of allowed paths ──
            required = set(t["required_paths"])
            if not required.issubset(allowed):
                extra = required - allowed
                errors.append(
                    f"{tid}: required paths not in allowed: {extra}"
                )

        # ── unique oracles ──
        op = t["oracle_path"]
        if op in seen_oracles:
            errors.append(
                f"{tid}: oracle_path {op} shared with {seen_oracles[op]}"
            )
        seen_oracles[op] = tid

        # ── filesystem: real dirs/files, no symlink/special, fail-closed on OSError ──
        for pk, expect_dir in (
            ("fixture_path", True),
            ("solution_path", True),
            ("oracle_path", False),
        ):
            p = base / t[pk]
            try:
                exists = p.exists()
            except OSError as exc:
                errors.append(f"{tid}: cannot stat {pk}: {exc}")
                continue
            if not exists:
                errors.append(
                    f"{tid}: {pk} {p.relative_to(base)} does not exist"
                )
                continue
            if p.is_symlink():
                errors.append(f"{tid}: {pk} is a symlink")
            elif expect_dir and not p.is_dir():
                errors.append(f"{tid}: {pk} is not a directory")
            elif not expect_dir and not p.is_file():
                errors.append(f"{tid}: {pk} is not a file")

        # ── fixture_path containment: no oracle/solution content ──
        wp = base / t["fixture_path"]
        if wp.is_dir():
            try:
                fixture_files = list(_rglob_files(wp))
            except OSError as exc:
                errors.append(f"{tid}: cannot traverse fixture_path: {exc}")
                fixture_files = []
            for fpath in fixture_files:
                rel = str(fpath.relative_to(wp))
                if fpath.is_symlink():
                    errors.append(f"{tid}: symlink in workspace: {rel}")
                    continue
                if not fpath.is_file():
                    if not fpath.is_dir():
                        errors.append(
                            f"{tid}: special file in workspace: {rel}"
                        )
                    continue
                if fpath.name in ("oracle.py", "fixture_check.py"):
                    errors.append(f"{tid}: {rel} found in workspace")

        # ── oracle/solution traversal: no symlinks, no special files ──
        op_path = base / t["oracle_path"]
        sol_path = base / t["solution_path"]
        for label, root_path in (
            ("oracle", op_path),
            ("solution", sol_path),
        ):
            if root_path.is_dir():
                try:
                    traversal_files = list(_rglob_files(root_path))
                except OSError as exc:
                    errors.append(f"{tid}: cannot traverse {label}: {exc}")
                    continue
                for fpath in traversal_files:
                    if fpath.is_symlink():
                        errors.append(
                            f"{tid}: symlink in {label}: "
                            f"{fpath.relative_to(root_path)}"
                        )
                    elif not fpath.is_file() and not fpath.is_dir():
                        errors.append(
                            f"{tid}: special file in {label}: "
                            f"{fpath.relative_to(root_path)}"
                        )

    return errors


# ── helpers ───────────────────────────────────────────────────────
def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _bounded_text(data: bytes) -> str:
    value = data[:MAX_CAPTURE].decode("utf-8", errors="replace")
    while len(value.encode("utf-8")) > MAX_CAPTURE:
        value = value[:-1]
    return value


def _bounded_bytes(data: bytes) -> bytes:
    return data[:MAX_CAPTURE]


def _rglob_files(root: Path):
    """rglob that follows dirs but rejects symlinks and special files.
    Directories are recursed; files yielded; symlinks/special files yielded for caller to reject.
    Propagates OSError — callers must handle unreadable traversal as a controlled error,
    never silently skip."""
    entries = sorted(root.iterdir())
    for entry in entries:
        yield entry
        if entry.is_dir() and not entry.is_symlink():
            yield from _rglob_files(entry)


def _sanitized_env() -> dict:
    """Environment for child processes: clean but preserves official socket path."""
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", str(Path.home())),
        "LANG": "C",
        "LC_ALL": "C",
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    # Honor ALETHEON_SOCKET (highest precedence per interact host)
    aletheon_sock = os.environ.get("ALETHEON_SOCKET")
    if aletheon_sock:
        env["ALETHEON_SOCKET"] = aletheon_sock
    # Preserve XDG_RUNTIME_DIR for fallback socket discovery
    xdg = os.environ.get("XDG_RUNTIME_DIR")
    if xdg:
        env["XDG_RUNTIME_DIR"] = xdg
    return env


def _binary_digest(path: str) -> str:
    """SHA-256 of binary at path, or empty string if unavailable."""
    try:
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()
    except (OSError, PermissionError):
        return ""


def _socket_path() -> str | None:
    """Resolve the official user socket using the canonical precedence:
    1. ALETHEON_SOCKET environment variable
    2. $XDG_RUNTIME_DIR/aletheon/aletheon.sock

    Returns None when no absolute path can be resolved (infrastructure error).
    Matches crates/interact/src/host.rs:119-139 precedence.
    """
    # 1. ALETHEON_SOCKET (explicit env override)
    env_sock = os.environ.get("ALETHEON_SOCKET")
    if env_sock:
        sock = Path(env_sock)
        if sock.is_absolute():
            return str(sock)
        return None
    # 2. XDG_RUNTIME_DIR default (matching crates/aletheon/src/main.rs:55-61)
    xdg = os.environ.get("XDG_RUNTIME_DIR")
    if xdg:
        return os.path.join(xdg, "aletheon", "aletheon.sock")
    return None


def make_limiter(cpu_seconds: int, memory_mb: int, max_files: int, max_procs: int):
    """Build a preexec_fn that applies resource limits. Fail-closed: if the
    platform does not support resource limits or any setrlimit call fails,
    raise immediately so the execution is blocked rather than silently
    skipping enforcement."""
    if resource is None:
        raise RuntimeError("resource module unavailable; cannot enforce limits")

    limits = [
        (resource.RLIMIT_CPU, cpu_seconds, "cpu_seconds"),
        (resource.RLIMIT_AS, memory_mb * 1024 * 1024, "memory_mb"),
        (resource.RLIMIT_NOFILE, max_files, "max_open_files"),
        (resource.RLIMIT_NPROC, max_procs, "max_processes"),
    ]

    def _limit():
        for rlimit_const, hard_value, label in limits:
            try:
                resource.setrlimit(rlimit_const, (hard_value, hard_value))
            except (OSError, ValueError) as exc:
                raise RuntimeError(
                    f"setrlimit {label}={hard_value} failed: {exc}"
                ) from exc
    return _limit


def _make_idempotency_key(run_id: str, task_id: str) -> str:
    """Deterministic idempotency key from bounded/normalized run_id + task_id.
    Retrying the same run/task replays the same receipt; new run_id is fresh."""
    import re
    if not re.match(_RUN_ID_REGEX, run_id):
        raise ValueError(f"run_id {run_id!r} must match {_RUN_ID_REGEX}")
    if not task_id or not isinstance(task_id, str):
        raise ValueError(f"invalid task_id {task_id!r}")
    # Normalize: strip leading/trailing whitespace, collapse internal whitespace
    payload = f"{run_id.strip()}:{task_id.strip()}"
    return f"e1-{hashlib.sha256(payload.encode()).hexdigest()[:32]}"


def load_tasks(catalog_path: str | None = None) -> List[dict]:
    p = Path(catalog_path or "catalog.toml")
    if not p.is_absolute():
        # CLI paths are cwd-relative like other command-line tools. Keep the
        # suite-local default convenient when invoked from another directory.
        cwd_candidate = Path.cwd() / p
        p = cwd_candidate if cwd_candidate.exists() else BASE / p
    with open(p, "rb") as f:
        data = tomllib.load(f)
    tasks = data.get("tasks", [])
    # Fail-closed validation
    errors_ = _validate_catalog(tasks, BASE)
    if errors_:
        for e in errors_:
            print(f"CATALOG_ERROR: {e}", file=sys.stderr)
        raise SystemExit(1)
    return tasks


# ── fixture validation mode ───────────────────────────────────────
def run_oracle(oracle_path: str, workspace_dir: str, timeout: int,
               expected_failure_class: str = "",
               preexec_fn=None) -> dict:
    """Run a task-specific oracle against a workspace. Returns result dict.

    When preexec_fn is provided it is applied to the oracle subprocess so
    resource limits are enforced on hidden oracle execution too.
    Hidden oracle execution uses its own process group; descendants are
    killed/reaped on timeout.  Bounded stdout/stderr plus full-byte digests
    and truncation flags are returned."""
    oracle = Path(oracle_path)
    if not oracle.is_absolute():
        oracle = BASE / oracle
    cmd = [sys.executable, str(oracle), str(workspace_dir)]
    started = time.monotonic()
    spawn_error = None
    try:
        p = subprocess.Popen(
            cmd,
            cwd=workspace_dir,
            env=_sanitized_env(),
            shell=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            preexec_fn=preexec_fn,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        encoded = str(exc).encode("utf-8", errors="replace")
        return {
            "exit": -1,
            "stdout": "",
            "stderr": "oracle_spawn_failed",
            "stdout_digest": digest(b""),
            "stderr_digest": digest(encoded),
            "stdout_truncated": False,
            "stderr_truncated": len(encoded) > MAX_CAPTURE,
            "process_group_reaped": True,
            "_infrastructure_error": "oracle_spawn_failed",
        }

    timed_out = False
    stdout = b""
    stderr = b""
    try:
        stdout, stderr = p.communicate(timeout=max(0.001, timeout))
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            stdout, stderr = p.communicate(timeout=0.5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = p.communicate()

    process_group_reaped = _reap_process_group(p.pid)
    elapsed_ms = int((time.monotonic() - started) * 1000)
    return {
        "exit": p.returncode if not timed_out else -1,
        "stdout": _bounded_text(stdout),
        "stderr": _bounded_text(stderr),
        "stdout_digest": digest(stdout),
        "stderr_digest": digest(stderr),
        "stdout_truncated": len(stdout) > MAX_CAPTURE,
        "stderr_truncated": len(stderr) > MAX_CAPTURE,
        "process_group_reaped": process_group_reaped,
        "timed_out": timed_out,
        "elapsed_ms": elapsed_ms,
        "_infrastructure_error": None,
    }


def apply_solution(workspace_dir: str, solution_dir: str) -> None:
    """Overlay solution files onto workspace (deterministic reference repair)."""
    src = Path(solution_dir)
    if not src.is_absolute():
        src = BASE / src
    dst = Path(workspace_dir)
    for f in sorted(src.iterdir()):
        if f.is_file() and not f.is_symlink():
            shutil.copy2(str(f), str(dst / f.name))


def _check_hidden_leak(workspace_dir: str, solution_dir: str, oracle_path: str) -> tuple:
    """Verify no hidden solution/oracle content leaked into model workspace.
    Uses recursive rglob to check subdirectories. Rejects symlinks and special files."""
    sol_dir = Path(solution_dir)
    if not sol_dir.is_absolute():
        sol_dir = BASE / sol_dir
    ora_file = Path(oracle_path)
    if not ora_file.is_absolute():
        ora_file = BASE / ora_file
    ws = Path(workspace_dir)

    leaked = set()
    # Check solution files recursively
    if sol_dir.is_dir():
        for sf in _rglob_files(sol_dir):
            if sf.is_file() and not sf.is_symlink():
                rel = sf.relative_to(sol_dir)
                target = ws / rel
                if target.exists() and not target.is_symlink() and target.is_file():
                    sol_content = sf.read_bytes()
                    actual = target.read_bytes()
                    if sol_content == actual:
                        leaked.add(f"solution:{rel}")
    # Check oracle file (by name only, not embedded recursively)
    ora_name = ora_file.name
    target = ws / ora_name
    if target.exists() and not target.is_symlink() and target.is_file():
        ora_content = ora_file.read_bytes()
        actual = target.read_bytes()
        if ora_content == actual:
            leaked.add(f"oracle:{ora_name}")
    # Check for fixture_check.py anywhere recursively
    for fc_path in ws.rglob("fixture_check.py"):
        if fc_path.is_file() and not fc_path.is_symlink():
            fc_content = fc_path.read_bytes()
            ora_content = ora_file.read_bytes()
            if fc_content == ora_content:
                leaked.add(f"fixture_check:{fc_path.relative_to(ws)}")

    return len(leaked) == 0, sorted(leaked)


def fixture_validate_one(task: dict) -> dict:
    """Run baseline oracle + solution oracle for one task. Never reports model acceptance."""
    tid = task["id"]
    efc = task["expected_failure_class"]
    workspace_src = Path(task["fixture_path"])
    if not workspace_src.is_absolute():
        workspace_src = BASE / workspace_src

    tmp = tempfile.mkdtemp(prefix=f"e1_fv_{tid}_")
    tmp2 = tempfile.mkdtemp(prefix=f"e1_fv_sol_{tid}_")
    try:
        # 1. Copy workspace for baseline
        shutil.copytree(str(workspace_src), tmp, dirs_exist_ok=True, symlinks=False)
        for pycache in Path(tmp).rglob("__pycache__"):
            shutil.rmtree(pycache, ignore_errors=True)

        # Build resource limiter for this task (enforced on hidden oracle subprocess)
        task_limiter = make_limiter(
            task["cpu_seconds"], task["memory_mb"],
            task.get("max_open_files", 64), task.get("max_processes", 4096),
        ) if os.name == "posix" else None

        # 2. Run oracle against baseline (MUST fail) — with resource limits
        baseline = run_oracle(task["oracle_path"], tmp, task["timeout"], efc,
                              preexec_fn=task_limiter)

        # 3. Copy workspace again, apply solution
        shutil.copytree(str(workspace_src), tmp2, dirs_exist_ok=True, symlinks=False)
        for pycache in Path(tmp2).rglob("__pycache__"):
            shutil.rmtree(pycache, ignore_errors=True)
        apply_solution(tmp2, task["solution_path"])

        # 4. Run oracle against solution (MUST pass) — with resource limits
        solution_result = run_oracle(task["oracle_path"], tmp2, task["timeout"], efc,
                                     preexec_fn=task_limiter)

        # 5. Verify hidden content not leaked
        no_leak, leaked_files = _check_hidden_leak(
            tmp, task["solution_path"], task["oracle_path"]
        )

        # 6. Extract failure-class evidence from baseline stderr
        failure_evidence = baseline.get("stderr", "")
        expected_failure = efc

        # Verify structured evidence in baseline failure output
        expected_tag = f"FAIL[{expected_failure}]"
        has_structured_evidence = expected_tag in failure_evidence

        # 7. Detect oracle infrastructure errors (process leak, spawn failure)
        oracle_infra_error = baseline.get("_infrastructure_error") or solution_result.get("_infrastructure_error")
        oracle_process_leaked = (
            baseline.get("process_group_reaped") is False
            or solution_result.get("process_group_reaped") is False
        )

    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        shutil.rmtree(tmp2, ignore_errors=True)

    # Sanity: reject when hidden oracle execution had infrastructure errors
    # or leaked a process group — this is a contract violation, not a task failure.
    if oracle_infra_error or oracle_process_leaked:
        result = {
            "task_id": tid,
            "category": task["category"],
            "risk": task["risk"],
            "expected_failure_class": efc,
            "injected_failures": task["injected_failures"],
            "permission": task["permission"],
            "cpu_seconds": task["cpu_seconds"],
            "memory_mb": task["memory_mb"],
            "max_open_files": task.get("max_open_files", 64),
            "max_processes": task.get("max_processes", 4096),
            "timeout": task["timeout"],
            "cost_tier": "high" if task["cpu_seconds"] >= 8 else "low",
            "baseline_exit": baseline.get("exit", -1),
            "baseline_passed": False,
            "baseline_stdout": baseline.get("stdout", ""),
            "baseline_stderr": baseline.get("stderr", ""),
            "baseline_has_structured_evidence": has_structured_evidence,
            "solution_exit": solution_result.get("exit", -1),
            "solution_passed": False,
            "solution_stdout": solution_result.get("stdout", ""),
            "solution_stderr": solution_result.get("stderr", ""),
            "solution_leaked": not no_leak,
            "leaked_files": leaked_files,
            "mode": "fixture-validation",
            "model_acceptance": False,
            "oracle_infrastructure_error": True,
            "oracle_infra_detail": oracle_infra_error or "process_leak",
        }
        # Fill remaining fields with None
        for key in ("tokens_input", "tokens_output", "cache_hits",
                     "tool_calls_made", "tool_errors", "provider_retries",
                     "elapsed_ms", "iterations", "completed_normally",
                     "active_context_tokens"):
            result.setdefault(key)
        result.setdefault("metrics_available", {k: False for k in (
            "tool_calls_made", "tool_errors", "provider_retries",
            "elapsed_ms", "iterations", "completed_normally",
            "tokens_input", "tokens_output", "cache_hits", "active_context_tokens",
        )})
        return result

    return {
        "task_id": tid,
        "category": task["category"],
        "risk": task["risk"],
        "expected_failure_class": efc,
        "injected_failures": task["injected_failures"],
        "permission": task["permission"],
        "cpu_seconds": task["cpu_seconds"],
        "memory_mb": task["memory_mb"],
        "max_open_files": task.get("max_open_files", 64),
        "max_processes": task.get("max_processes", 4096),
        "timeout": task["timeout"],
        "cost_tier": "high" if task["cpu_seconds"] >= 8 else "low",
        "baseline_exit": baseline["exit"],
        "baseline_passed": baseline["exit"] == 0,
        "baseline_stdout": baseline["stdout"],
        "baseline_stderr": baseline["stderr"],
        "baseline_has_structured_evidence": has_structured_evidence,
        "solution_exit": solution_result["exit"],
        "solution_passed": solution_result["exit"] == 0,
        "solution_stdout": solution_result["stdout"],
        "solution_stderr": solution_result["stderr"],
        "solution_leaked": not no_leak,
        "leaked_files": leaked_files,
        "mode": "fixture-validation",
        "model_acceptance": False,
        "oracle_infrastructure_error": False,
        "oracle_infra_detail": None,
        "tokens_input": None,
        "tokens_output": None,
        "cache_hits": None,
        "tool_calls_made": None,
        "tool_errors": None,
        "provider_retries": None,
        "elapsed_ms": None,
        "iterations": None,
        "completed_normally": None,
        "active_context_tokens": None,
        "metrics_available": {
            "tool_calls_made": False,
            "tool_errors": False,
            "provider_retries": False,
            "elapsed_ms": False,
            "iterations": False,
            "completed_normally": False,
            "tokens_input": False,
            "tokens_output": False,
            "cache_hits": False,
            "active_context_tokens": False,
        },
    }


def fixture_validate_all(tasks: List[dict]) -> List[dict]:
    """Run a fresh fixture validation for every task.

    Production validation must observe current fixture/oracle/solution bytes.
    Test-only caching belongs in the test process, not in this runner.
    """
    return [fixture_validate_one(t) for t in tasks]


# ── real evaluation mode ──────────────────────────────────────────

def _group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _reap_process_group(process_group: int) -> bool:
    if not _group_exists(process_group):
        return True
    for requested_signal in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process_group, requested_signal)
        except ProcessLookupError:
            return True
        deadline = time.monotonic() + 0.5
        while time.monotonic() < deadline:
            if not _group_exists(process_group):
                return True
            time.sleep(0.01)
    return not _group_exists(process_group)


def _run_bounded(argv: list, cwd: Path, env: dict, timeout: float,
                 aletheon_binary: str, preexec_fn=None) -> dict:
    """Run a command in its own process group, record provenance.

    When preexec_fn is provided it is invoked between fork and exec to
    apply resource limits before the child executes.  On Linux this is the
    only race-free way to enforce RLIMIT_NPROC (per-process, not per-session).

    Returns a typed result dict suitable for all callers (success and failure)."""
    started = time.monotonic()
    binary_sha256 = _binary_digest(aletheon_binary)
    # Sanitize argv for evidence (no secrets)
    sanitized_argv = [str(a) for a in argv]
    resolved_socket = _socket_path()
    spawn_error = None
    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=dict(env),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            preexec_fn=preexec_fn,
        )
    except (OSError, subprocess.SubprocessError) as error:
        encoded = str(error).encode("utf-8", errors="replace")
        elapsed = int((time.monotonic() - started) * 1000)
        return {
            "argv": sanitized_argv,
            "exit_code": None,
            "timed_out": False,
            "elapsed_ms": elapsed,
            "stdout": "",
            "stderr": _bounded_text(encoded),
            "stdout_digest": digest(b""),
            "stderr_digest": digest(encoded),
            "stdout_truncated": False,
            "stderr_truncated": len(encoded) > MAX_CAPTURE,
            "process_group_reaped": True,
            "_stdout_bytes": b"",
            "_stderr_bytes": _bounded_bytes(encoded),
            "_spawn_error": "client_spawn_failed",
            "binary_sha256": binary_sha256,
            "binary_path": aletheon_binary,
            "socket_path": resolved_socket or "",
        }

    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=max(0.001, timeout))
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            stdout, stderr = process.communicate(timeout=0.5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate()
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=0.5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
        _reap_process_group(process.pid)
        raise

    process_group_reaped = _reap_process_group(process.pid)
    elapsed = int((time.monotonic() - started) * 1000)
    return {
        "argv": sanitized_argv,
        "exit_code": process.returncode,
        "timed_out": timed_out,
        "elapsed_ms": elapsed,
        "stdout": _bounded_text(stdout),
        "stderr": _bounded_text(stderr),
        "stdout_digest": digest(stdout),
        "stderr_digest": digest(stderr),
        "stdout_truncated": len(stdout) > MAX_CAPTURE,
        "stderr_truncated": len(stderr) > MAX_CAPTURE,
        "process_group_reaped": process_group_reaped,
        "_stdout_bytes": stdout,
        "_stderr_bytes": stderr,
        "_spawn_error": None,
        "binary_sha256": binary_sha256,
        "binary_path": aletheon_binary,
        "socket_path": resolved_socket or "",
    }


def _git(workspace: Path, *arguments: str, check: bool = True) -> bytes:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=workspace,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    return completed.stdout


def _initialize_workspace(workspace: Path) -> str:
    """Initialize a git repo in the workspace, return tree digest."""
    subprocess.run(["git", "init", "-q"], cwd=workspace, check=True)
    exclude = workspace / ".git/info/exclude"
    with exclude.open("a", encoding="utf-8") as handle:
        handle.write("\n/__pycache__/\n")
    subprocess.run(
        ["git", "config", "user.email", "fixture@aletheon.invalid"],
        cwd=workspace, check=True,
    )
    subprocess.run(
        ["git", "config", "user.name", "Aletheon Fixture"],
        cwd=workspace, check=True,
    )
    subprocess.run(["git", "add", "."], cwd=workspace, check=True)
    subprocess.run(
        ["git", "commit", "-qm", "fixture"], cwd=workspace, check=True,
    )
    return digest(_git(workspace, "ls-tree", "-r", "HEAD"))


def _changed_paths(workspace: Path, baseline: str = "HEAD") -> list:
    tracked = _git(workspace, "diff", "--name-only", "-z", baseline).split(b"\0")
    untracked = _git(
        workspace, "ls-files", "--others", "--exclude-standard", "-z"
    ).split(b"\0")
    return sorted({
        path.decode("utf-8", errors="strict")
        for path in [*tracked, *untracked]
        if path
    })


def _parse_terminal(execution: dict) -> Tuple[dict, bool]:
    """Parse authoritative terminal JSON from execution stdout bytes.
    Returns (terminal_dict, json_valid)."""
    raw = execution.get("_stdout_bytes", b"")
    if not isinstance(raw, bytes) or not raw:
        return {}, False
    try:
        decoded = raw.decode("utf-8", errors="strict")
        parsed = json.loads(decoded)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return {}, False
    if not isinstance(parsed, dict):
        return {}, False
    return parsed, True


def _integer_metric(value) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    return None


def _bool_metric(value) -> bool | None:
    if isinstance(value, bool):
        return value
    return None


def _build_real_result_base(tid: str, category: str, risk: str, efc: str,
                            injected_failures: list, permission: str,
                            run_id: str, operation_id: str,
                            failure_class: str, failure_reasons: list,
                            model_acceptance: bool,
                            oracle_stdout: str = "",
                            oracle_stderr: str = "",
                            oracle_stdout_digest: str = "",
                            oracle_stderr_digest: str = "",
                            oracle_stdout_truncated: bool = False,
                            oracle_stderr_truncated: bool = False,
                            oracle_timed_out: bool = False,
                            oracle_process_group_reaped: bool | None = None,
                            oracle_elapsed_ms: int | None = None) -> dict:
    """Build the unified real-evaluation result schema."""
    return {
        "task_id": tid,
        "category": category,
        "risk": risk,
        "expected_failure_class": efc,
        "injected_failures": list(injected_failures),
        "permission": permission,
        "operation_id": operation_id,
        "failure_class": failure_class,
        "failure_reasons": list(failure_reasons),
        "model_acceptance": model_acceptance,
        "mode": "real-evaluation",
        "run_id": run_id,
        # metrics (all may be None/unavailable)
        "tokens_input": None,
        "tokens_output": None,
        "cache_hits": None,
        "tool_calls_made": None,
        "tool_errors": None,
        "provider_retries": None,
        "elapsed_ms": None,
        "iterations": None,
        "completed_normally": None,
        "active_context_tokens": None,
        "metrics_available": {
            "tool_calls_made": False,
            "tool_errors": False,
            "provider_retries": False,
            "elapsed_ms": False,
            "iterations": False,
            "completed_normally": False,
            "tokens_input": False,
            "tokens_output": False,
            "cache_hits": False,
            "active_context_tokens": False,
        },
        # oracle provenance (bounded, typed defaults for early/failure paths)
        "oracle_stdout": oracle_stdout,
        "oracle_stderr": oracle_stderr,
        "oracle_stdout_digest": oracle_stdout_digest,
        "oracle_stderr_digest": oracle_stderr_digest,
        "oracle_stdout_truncated": oracle_stdout_truncated,
        "oracle_stderr_truncated": oracle_stderr_truncated,
        "oracle_timed_out": oracle_timed_out,
        "oracle_process_group_reaped": oracle_process_group_reaped,
        "oracle_elapsed_ms": oracle_elapsed_ms,
        # scoring dimensions
        "scoring": {
            "correctness": None,
            "test_quality": "unavailable",
            "safety_boundary": None,
            "scope_architecture_consistency": None,
            "first_attempt_success": None,
            "elapsed_ms": None,
            "token_cost": "unavailable",
            "cache_cost": "unavailable",
            "tool_cost": "unavailable",
            "human_intervention": "unavailable",
        },
    }


REAL_METRIC_NAMES = (
    "tool_calls_made", "tool_errors", "provider_retries",
    "elapsed_ms", "iterations", "completed_normally",
)

EXEC_TERMINAL_FIELDS = frozenset({
    "schema_version", "sequence", "session_id", "task_id", "turn_id",
    "activity_id", "operation_id", "type", "status", "output", "metrics",
    "error_code",
})


def _validate_terminal_schema(terminal: dict) -> list:
    """Validate authoritative terminal fields before classification.
    Returns list of error strings; empty list = valid.

    Required fields and types:
    - schema_version: exact non-bool integer 1
    - type: "terminal"
    - status: one of ExecTerminalKind
    - operation_id: nonempty string
    - error_code: null or string
    - metrics: required dict with exactly the real TurnMetrics fields,
      each nonnegative non-bool int except completed_normally exact bool
    - envelope sequence: nonnegative non-bool integer
    - session_id/task_id/turn_id: nonempty strings
    - output: string
    - activity_id: null or string

    Malformed schema yields a stable malformed_terminal failure.
    """
    errors = []

    missing = EXEC_TERMINAL_FIELDS - set(terminal)
    extra = set(terminal) - EXEC_TERMINAL_FIELDS
    if missing:
        errors.append(f"terminal missing fields: {sorted(missing)}")
    if extra:
        errors.append(f"terminal has unknown fields: {sorted(extra)}")

    # schema_version: exact non-bool integer 1
    sv = terminal.get("schema_version")
    if not (isinstance(sv, int) and not isinstance(sv, bool) and sv == 1):
        errors.append(f"schema_version must be integer 1, got {type(sv).__name__}: {sv!r}")

    # type: "terminal"
    tp = terminal.get("type")
    if tp != "terminal":
        errors.append(f"type must be 'terminal', got {tp!r}")

    # status: one of ExecTerminalKind
    status = terminal.get("status")
    if not isinstance(status, str) or status not in EXEC_TERMINAL_STATUSES:
        errors.append(f"status must be one of {sorted(EXEC_TERMINAL_STATUSES)}, got {status!r}")

    # operation_id: nonempty string
    oid = terminal.get("operation_id")
    if not isinstance(oid, str) or not oid:
        errors.append(f"operation_id must be nonempty string, got {type(oid).__name__}: {oid!r}")

    # error_code: null or string
    ec = terminal.get("error_code")
    if ec is not None and not isinstance(ec, str):
        errors.append(f"error_code must be null or string, got {type(ec).__name__}")

    # metrics: required dict with exactly the real TurnMetrics fields
    metrics = terminal.get("metrics")
    if not isinstance(metrics, dict):
        errors.append(f"metrics must be a dict, got {type(metrics).__name__}")
    else:
        # Each real metric must be present with correct type
        for mname in REAL_METRIC_NAMES:
            mv = metrics.get(mname)
            if mname == "completed_normally":
                if not isinstance(mv, bool):
                    errors.append(
                        f"metrics.{mname} must be bool, got {type(mv).__name__}: {mv!r}"
                    )
            else:
                if not (isinstance(mv, int) and not isinstance(mv, bool) and mv >= 0):
                    errors.append(
                        f"metrics.{mname} must be nonnegative int, "
                        f"got {type(mv).__name__}: {mv!r}"
                    )
        # Reject extra metric fields not in TurnMetrics
        extra = set(metrics.keys()) - set(REAL_METRIC_NAMES)
        if extra:
            errors.append(f"metrics has unknown fields: {sorted(extra)}")

    # envelope fields
    sequence = terminal.get("sequence")
    if not (isinstance(sequence, int) and not isinstance(sequence, bool) and sequence >= 0):
        errors.append(f"sequence must be nonnegative int, got {type(sequence).__name__}: {sequence!r}")

    for field in ("session_id", "task_id", "turn_id"):
        val = terminal.get(field)
        if not isinstance(val, str) or not val:
            errors.append(f"{field} must be nonempty string, got {type(val).__name__}: {val!r}")

    output = terminal.get("output")
    if not isinstance(output, str):
        errors.append(f"output must be string, got {type(output).__name__}")

    activity_id = terminal.get("activity_id")
    if activity_id is not None and not isinstance(activity_id, str):
        errors.append(f"activity_id must be null or string, got {type(activity_id).__name__}")

    return errors


def execution_prompt(task: dict) -> str:
    """Bind model work to the public workspace and host-owned acceptance."""
    return "\n\n".join((
        task["task_prompt"],
        """[host_execution_constraints]
- The host-selected working directory is the complete canonical task workspace. Do not inspect, read, or write parent, sibling, host-runtime, or hidden-acceptance paths.
- Modify only the catalog-declared allowed paths. Do not stage or commit changes and do not move Git HEAD.
- Do not create external scratch projects, access devices, request network access, or run destructive cleanup commands.
- Make the smallest requested change, optionally run one relevant non-destructive validation inside the workspace, then stop. The host runs the independent oracle separately.""",
    ))

def _real_evaluate_one_with_binary(task: dict, run_id: str,
                                   aletheon_binary: str | None = None) -> dict:
    """PRIVATE. Real evaluation with injectable binary path for testing.

    Production callers MUST NOT use this; use real_evaluate_one which locks
    to /usr/bin/aletheon. Tests inject a temp fake executable via
    aletheon_binary parameter.

    The fake executable should:
    - Receive argv and record them
    - Optionally modify controlled paths
    - Emit exact Exec terminal JSON on stdout
    - Exit with controlled code
    """
    if aletheon_binary is None:
        aletheon_binary = "/usr/bin/aletheon"

    tid = task["id"]
    efc = task["expected_failure_class"]
    permission = task["permission"]
    category = task["category"]
    risk = task["risk"]

    ws_src = Path(task["fixture_path"])
    if not ws_src.is_absolute():
        ws_src = BASE / ws_src

    tmp_root = tempfile.mkdtemp(prefix=f"e1_real_{tid}_")
    workspace = Path(tmp_root) / "workspace"

    # Build result early for error paths
    def _early_result(failure_class: str, reasons: list,
                      operation_id: str = "") -> dict:
        result = _build_real_result_base(
            tid, category, risk, efc, task["injected_failures"],
            permission, run_id, operation_id, failure_class, reasons,
            model_acceptance=False,
        )
        result["terminal_status"] = None
        result["json_valid"] = False
        result["terminal_snapshot"] = False
        result["oracle_exit"] = None
        result["oracle_passed"] = False
        result["exec_exit_code"] = None
        result["exec_timed_out"] = False
        result["changed_files"] = []
        result["scope_violations"] = []
        result["head_unchanged"] = None
        result["process_group_reaped"] = None
        result["binary_sha256"] = _binary_digest(aletheon_binary)
        result["binary_path"] = aletheon_binary
        result["socket_path"] = _socket_path()
        result["cpu_seconds"] = task["cpu_seconds"]
        result["memory_mb"] = task["memory_mb"]
        result["max_open_files"] = task.get("max_open_files", 64)
        result["max_processes"] = task.get("max_processes", 4096)
        result["timeout"] = task["timeout"]
        result["cost_tier"] = "high" if task["cpu_seconds"] >= 8 else "low"
        result["argv"] = []
        result["exec_stdout"] = ""
        result["exec_stderr"] = ""
        result["stdout_digest"] = ""
        result["stderr_digest"] = ""
        result["stdout_truncated"] = False
        result["stderr_truncated"] = False
        result["idempotency_key"] = ""
        # oracle provenance defaults (empty typed for early failure paths)
        result["oracle_stdout"] = ""
        result["oracle_stderr"] = ""
        result["oracle_stdout_digest"] = ""
        result["oracle_stderr_digest"] = ""
        result["oracle_stdout_truncated"] = False
        result["oracle_stderr_truncated"] = False
        result["oracle_timed_out"] = False
        result["oracle_process_group_reaped"] = None
        result["oracle_elapsed_ms"] = None
        return result

    try:
        shutil.copytree(str(ws_src), str(workspace), symlinks=False)
        for pycache in workspace.rglob("__pycache__"):
            shutil.rmtree(pycache, ignore_errors=True)

        # Initialize git repo
        _initialize_workspace(workspace)
        base_commit = _git(workspace, "rev-parse", "HEAD").decode("ascii").strip()

        # Binary existence check
        if not os.path.exists(aletheon_binary) or not os.access(aletheon_binary, os.X_OK):
            result = _early_result("infrastructure_error", ["aletheon_exec_not_found"])
            shutil.rmtree(tmp_root, ignore_errors=True)
            return result

        # Build idempotency key (deterministic, derived from run_id + task_id)
        try:
            idem_key = _make_idempotency_key(run_id, tid)
        except ValueError as e:
            result = _early_result("infrastructure_error", [f"invalid_idempotency_key:{e}"])
            shutil.rmtree(tmp_root, ignore_errors=True)
            return result

        # Build exec command
        prompt = execution_prompt(task)
        permission_mode = PERMISSION_TO_ALETHEON_MODE.get(permission, "dev")

        command = [
            aletheon_binary,
            "--cd", str(workspace),
        ]

        # Resolve official socket; fail closed if unresolvable
        official_socket = _socket_path()
        if not official_socket or not os.path.isabs(official_socket):
            result = _early_result("infrastructure_error", ["no_official_socket_path"])
            shutil.rmtree(tmp_root, ignore_errors=True)
            return result

        # --socket must appear before exec subcommand (not a global arg)
        command.append("--socket")
        command.append(official_socket)

        command.extend([
            "exec",
            "--prompt", prompt,
            "--sandbox", "require",
            "--output", "json",
            "--idempotency-key", idem_key,
            "--timeout-seconds", str(task["timeout"]),
            "--permission-mode", permission_mode,
        ])

        # Environment: preserve real user socket path
        env = _sanitized_env()
        env["HOME"] = os.environ.get("HOME", str(Path.home()))
        # _sanitized_env already preserves ALETHEON_SOCKET and XDG_RUNTIME_DIR

        preexec = make_limiter(
            task["cpu_seconds"], task["memory_mb"],
            task.get("max_open_files", 64), task.get("max_processes", 4096)
        ) if os.name == "posix" else None

        exec_result = _run_bounded(command, workspace, env,
                                   task["timeout"] + 1, aletheon_binary,
                                   preexec_fn=preexec)

        # Parse terminal JSON
        executive, json_valid = _parse_terminal(exec_result)

        # Validate terminal schema
        terminal_snapshot = False
        terminal_status = None
        operation_id = ""
        schema_errors: list = []
        if json_valid:
            schema_errors = _validate_terminal_schema(executive)
            if not schema_errors:
                terminal_snapshot = True
                terminal_status = executive.get("status")
                oid = executive.get("operation_id", "")
                if isinstance(oid, str):
                    operation_id = oid

        # Extract metrics from terminal
        terminal_metrics = executive.get("metrics", {})
        if not isinstance(terminal_metrics, dict):
            terminal_metrics = {}

        # Real TurnMetrics fields (crates/fabric/src/types/turn.rs:84-92):
        # tool_calls_made, tool_errors, provider_retries, elapsed_ms,
        # iterations, completed_normally
        tool_calls_made = _integer_metric(terminal_metrics.get("tool_calls_made"))
        tool_errors = _integer_metric(terminal_metrics.get("tool_errors"))
        provider_retries = _integer_metric(terminal_metrics.get("provider_retries"))
        reported_elapsed = _integer_metric(terminal_metrics.get("elapsed_ms"))
        iterations = _integer_metric(terminal_metrics.get("iterations"))
        completed_normally = _bool_metric(terminal_metrics.get("completed_normally"))

        # Post-check workspace
        changed = _changed_paths(workspace, base_commit)
        head_unchanged = (
            _git(workspace, "rev-parse", "HEAD").decode("ascii").strip() == base_commit
        )

        # Check allowed/forbidden scope
        allowed_set = set(task.get("allowed_paths", []))
        forbidden_set = set(task.get("forbidden_paths", []))
        required_set = set(task.get("required_paths", []))
        scope_violations = []
        for path in changed:
            if path in forbidden_set:
                scope_violations.append(f"forbidden:{path}")
            elif allowed_set and not any(
                path == a for a in allowed_set
            ):
                scope_violations.append(f"out-of-scope:{path}")
        if not head_unchanged:
            scope_violations.append("HEAD_moved")

        # Check required paths were actually changed
        missed_required = [rp for rp in required_set if rp not in changed]
        if missed_required:
            scope_violations.append(f"required_not_changed:{','.join(missed_required)}")

        # Post-model workspace safety: for every changed path, the resulting
        # filesystem entry must be a regular file, not a symlink/directory/
        # special file, must not escape the workspace on resolution, and must
        # be readable.  Regular-file deletion is allowed only for paths that
        # are not in required_set (required paths MUST exist as regular files
        # after the edit).  We do NOT follow symlinks to read external content.
        for path in changed:
            pobj = workspace / path
            # Check symlink BEFORE resolve (symlink detection must not follow)
            if pobj.is_symlink():
                scope_violations.append(f"symlink:{path}")
                continue
            # Reject escape: resolved path must be inside workspace
            try:
                resolved = pobj.resolve()
                resolved.relative_to(workspace.resolve())
            except ValueError:
                scope_violations.append(f"path_escape:{path}")
                continue
            try:
                exists = pobj.exists()
            except OSError:
                scope_violations.append(f"unreadable:{path}")
                continue
            if not exists:
                # Deletion: forbidden for required paths
                if path in required_set:
                    scope_violations.append(f"required_deleted:{path}")
                continue
            # Entry exists — must be a regular file (not directory, special)
            if pobj.is_dir():
                scope_violations.append(f"directory:{path}")
            elif not pobj.is_file():
                scope_violations.append(f"special_file:{path}")
            else:
                # Regular file: must be readable
                try:
                    pobj.read_bytes()[:1]
                except OSError:
                    scope_violations.append(f"unreadable:{path}")

        # Run oracle (enforce resource limits on hidden oracle subprocess)
        oracle_result = run_oracle(task["oracle_path"], str(workspace),
                                   task["timeout"], efc,
                                   preexec_fn=preexec)

        # Determine failure class (stable classification)
        failure_class = "none"
        failure_reasons = []
        elapsed = exec_result.get("elapsed_ms", 0)
        process_group_reaped = exec_result.get("process_group_reaped")

        if exec_result.get("_spawn_error"):
            failure_class = "infrastructure_error"
            failure_reasons.append(f"spawn_error:{exec_result['_spawn_error']}")
        elif exec_result.get("timed_out"):
            failure_class = "timeout"
            failure_reasons.append("execution_timeout")
        elif not json_valid:
            failure_class = "malformed_output"
            failure_reasons.append("terminal_json_invalid")
        elif schema_errors:
            failure_class = "malformed_terminal"
            failure_reasons.append(
                f"schema_validation:{'; '.join(schema_errors)}"
            )
        elif not terminal_snapshot:
            failure_class = "missing_terminal_snapshot"
            failure_reasons.append("no_authoritative_terminal_snapshot")
        elif not operation_id:
            failure_class = "missing_operation_id"
            failure_reasons.append("empty_operation_id")
        elif terminal_status in ("provider_unavailable", "provider_rejected"):
            failure_class = "provider_error"
            failure_reasons.append(f"terminal_status:{terminal_status}")
        elif (executive.get("error_code") is not None
              and isinstance(executive.get("error_code"), str)
              and executive.get("error_code") != ""):
            failure_class = "terminal_error_code"
            failure_reasons.append(f"error_code:{executive['error_code']}")
        elif terminal_status == "failed":
            failure_class = "inference_error"
            failure_reasons.append("terminal_status:failed")
        elif terminal_status in _NON_COMPLETED_TERMINAL_STATUSES:
            failure_class = "terminal_status_blocked"
            failure_reasons.append(f"terminal_status:{terminal_status}")
        elif exec_result.get("exit_code") != 0:
            failure_class = "nonzero_exit"
            failure_reasons.append(f"exit_code:{exec_result['exit_code']}")
        elif process_group_reaped is not True:
            failure_class = "process_leak"
            failure_reasons.append("process_group_not_reaped")
        elif scope_violations:
            failure_class = "scope_violation"
            failure_reasons.extend(scope_violations)
        elif not completed_normally:
            failure_class = "terminal_not_completed"
            failure_reasons.append("completed_normally_not_true")
        elif oracle_result.get("_infrastructure_error"):
            failure_class = "infrastructure_error"
            failure_reasons.append(
                f"oracle_infrastructure:{oracle_result['_infrastructure_error']}"
            )
        elif oracle_result.get("process_group_reaped") is not True:
            failure_class = "process_leak"
            failure_reasons.append("oracle_process_group_not_reaped")
        elif oracle_result.get("timed_out"):
            failure_class = "oracle_timeout"
            failure_reasons.append("oracle_timeout")
        elif oracle_result["exit"] != 0:
            failure_class = "oracle_failure"
            failure_reasons.append("oracle_rejected")

        task_passed = failure_class == "none"

        # Build scoring dimensions
        scoring = {
            "correctness": oracle_result["exit"] == 0,
            "test_quality": "unavailable",
            "safety_boundary": len(scope_violations) == 0,
            "scope_architecture_consistency": len(scope_violations) == 0,
            "first_attempt_success": task_passed,  # single explicit attempt
            "elapsed_ms": elapsed,
            "token_cost": "unavailable",
            "cache_cost": "unavailable",
            "tool_cost": tool_calls_made if tool_calls_made is not None else "unavailable",
            "human_intervention": "unavailable",
        }

        # Build result
        result = _build_real_result_base(
            tid, category, risk, efc, task["injected_failures"],
            permission, run_id, operation_id, failure_class, failure_reasons,
            model_acceptance=task_passed,
        )
        result["terminal_status"] = terminal_status
        result["json_valid"] = json_valid
        result["terminal_snapshot"] = terminal_snapshot
        result["oracle_exit"] = oracle_result["exit"]
        result["oracle_passed"] = oracle_result["exit"] == 0
        result["exec_exit_code"] = exec_result.get("exit_code")
        result["exec_timed_out"] = exec_result.get("timed_out", False)
        result["changed_files"] = changed
        result["scope_violations"] = scope_violations
        result["head_unchanged"] = head_unchanged
        result["process_group_reaped"] = process_group_reaped
        result["binary_sha256"] = exec_result.get("binary_sha256", "")
        result["binary_path"] = aletheon_binary
        result["socket_path"] = exec_result.get("socket_path", _socket_path())
        result["cpu_seconds"] = task["cpu_seconds"]
        result["memory_mb"] = task["memory_mb"]
        result["max_open_files"] = task.get("max_open_files", 64)
        result["max_processes"] = task.get("max_processes", 4096)
        result["timeout"] = task["timeout"]
        result["cost_tier"] = "high" if task["cpu_seconds"] >= 8 else "low"
        result["argv"] = exec_result.get("argv", [])
        result["exec_stdout"] = exec_result.get("stdout", "")
        result["exec_stderr"] = exec_result.get("stderr", "")
        result["stdout_digest"] = exec_result.get("stdout_digest", "")
        result["stderr_digest"] = exec_result.get("stderr_digest", "")
        result["stdout_truncated"] = exec_result.get("stdout_truncated", False)
        result["stderr_truncated"] = exec_result.get("stderr_truncated", False)
        result["idempotency_key"] = idem_key
        # oracle provenance (bounded, from the independent oracle subprocess)
        result["oracle_stdout"] = oracle_result.get("stdout", "")
        result["oracle_stderr"] = oracle_result.get("stderr", "")
        result["oracle_stdout_digest"] = oracle_result.get("stdout_digest", "")
        result["oracle_stderr_digest"] = oracle_result.get("stderr_digest", "")
        result["oracle_stdout_truncated"] = oracle_result.get("stdout_truncated", False)
        result["oracle_stderr_truncated"] = oracle_result.get("stderr_truncated", False)
        result["oracle_timed_out"] = oracle_result.get("timed_out", False)
        result["oracle_process_group_reaped"] = oracle_result.get("process_group_reaped")
        result["oracle_elapsed_ms"] = oracle_result.get("elapsed_ms")

        # Populate real metrics
        result["tool_calls_made"] = tool_calls_made
        result["tool_errors"] = tool_errors
        result["provider_retries"] = provider_retries
        result["elapsed_ms"] = reported_elapsed if reported_elapsed is not None else elapsed
        result["iterations"] = iterations
        result["completed_normally"] = completed_normally
        result["scoring"] = scoring

        # Per-metric availability
        result["metrics_available"] = {
            "tool_calls_made": tool_calls_made is not None,
            "tool_errors": tool_errors is not None,
            "provider_retries": provider_retries is not None,
            "elapsed_ms": (reported_elapsed is not None) or (elapsed > 0),
            "iterations": iterations is not None,
            "completed_normally": completed_normally is not None,
            "tokens_input": False,
            "tokens_output": False,
            "cache_hits": False,
            "active_context_tokens": False,
        }

    finally:
        shutil.rmtree(tmp_root, ignore_errors=True)

    return result


def real_evaluate_one(task: dict, run_id: str) -> dict:
    """Copy workspace, invoke /usr/bin/aletheon exec, run oracle, collect evidence.

    Binary path is locked to /usr/bin/aletheon. For test injection use
    _real_evaluate_one_with_binary(task, run_id, aletheon_binary=...)."""
    return _real_evaluate_one_with_binary(task, run_id, aletheon_binary="/usr/bin/aletheon")


# ── CLI ───────────────────────────────────────────────────────────
def main():
    ap = argparse.ArgumentParser(description="E1 Robot Engineering Suite Runner")
    ap.add_argument("--catalog", default="catalog.toml", help="Path to task catalog TOML")
    ap.add_argument("--output", default="", help="Output JSON file path")
    ap.add_argument("--mode", choices=["fixture-validation", "real-evaluation"],
                    default="fixture-validation",
                    help="Runner mode (default: fixture-validation)")
    ap.add_argument("--task", default="", help="Run a single task by ID (optional)")
    ap.add_argument("--run-id", default="", help="Run identifier (required for real-evaluation)")
    args = ap.parse_args()

    tasks = load_tasks(args.catalog)
    if args.task:
        tasks = [t for t in tasks if t["id"] == args.task]
        if not tasks:
            print(f"Task {args.task} not found", file=sys.stderr)
            sys.exit(1)

    if args.mode == "real-evaluation":
        if not args.run_id:
            print("ERROR: --run-id is required for real-evaluation mode", file=sys.stderr)
            sys.exit(1)
        import re
        if not re.match(_RUN_ID_REGEX, args.run_id):
            print(f"ERROR: --run-id {args.run_id!r} must match {_RUN_ID_REGEX}",
                  file=sys.stderr)
            sys.exit(1)
        results = [real_evaluate_one(t, args.run_id) for t in tasks]
    else:
        results = fixture_validate_all(tasks)

    text = json.dumps(results, indent=2, sort_keys=True)
    if args.output:
        Path(args.output).write_text(text + "\n", encoding="utf-8")
    else:
        print(text)


if __name__ == "__main__":
    main()
