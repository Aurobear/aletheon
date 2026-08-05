#!/usr/bin/env python3
"""Run one strict coding task through the real ``aletheon exec`` boundary."""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import signal
import subprocess
import tempfile
import time
from collections.abc import Mapping

from contracts import BenchmarkTask, load_task
from receipt import classify_failure, digest, seal

ROOT = pathlib.Path(__file__).resolve().parents[3]
MAX_CAPTURE = 64 * 1024
_COMMAND_FIELDS = (
    "argv",
    "exit_code",
    "timed_out",
    "elapsed_ms",
    "stdout",
    "stderr",
    "stdout_digest",
    "stderr_digest",
    "stdout_truncated",
    "stderr_truncated",
    "process_group_reaped",
)


def _bounded_text(data: bytes) -> str:
    value = data[:MAX_CAPTURE].decode("utf-8", errors="replace")
    while len(value.encode("utf-8")) > MAX_CAPTURE:
        value = value[:-1]
    return value


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


def run_bounded(
    argv: list[str], cwd: pathlib.Path, env: Mapping[str, str], timeout: float
) -> dict[str, object]:
    """Run a command in its own process group and retain private full bytes."""
    started = time.monotonic()
    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=dict(env),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except OSError as error:
        encoded = str(error).encode("utf-8", errors="replace")
        return {
            "argv": list(argv),
            "exit_code": None,
            "timed_out": False,
            "elapsed_ms": int((time.monotonic() - started) * 1000),
            "stdout": "",
            "stderr": _bounded_text(encoded),
            "stdout_digest": digest(b""),
            "stderr_digest": digest(encoded),
            "stdout_truncated": False,
            "stderr_truncated": len(encoded) > MAX_CAPTURE,
            "process_group_reaped": True,
            "_stdout_bytes": b"",
            "_stderr_bytes": encoded,
            "_spawn_error": "client_spawn_failed",
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
    return {
        "argv": list(argv),
        "exit_code": process.returncode,
        "timed_out": timed_out,
        "elapsed_ms": int((time.monotonic() - started) * 1000),
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
    }


def _deadline_result(argv: list[str]) -> dict[str, object]:
    return {
        "argv": argv,
        "exit_code": None,
        "timed_out": True,
        "elapsed_ms": 0,
        "stdout": "",
        "stderr": "task deadline exhausted before command start",
        "stdout_digest": digest(b""),
        "stderr_digest": digest(b"task deadline exhausted before command start"),
        "stdout_truncated": False,
        "stderr_truncated": False,
        "process_group_reaped": True,
        "_stdout_bytes": b"",
        "_stderr_bytes": b"task deadline exhausted before command start",
        "_spawn_error": None,
    }


def _public_command(result: Mapping[str, object]) -> dict[str, object]:
    return {field: result[field] for field in _COMMAND_FIELDS}


def _git(workspace: pathlib.Path, *arguments: str, check: bool = True) -> bytes:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=workspace,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    return completed.stdout


def _initialize_workspace(workspace: pathlib.Path) -> str:
    subprocess.run(["git", "init", "-q"], cwd=workspace, check=True)
    subprocess.run(
        ["git", "config", "user.email", "fixture@aletheon.invalid"],
        cwd=workspace,
        check=True,
    )
    subprocess.run(
        ["git", "config", "user.name", "Aletheon Fixture"],
        cwd=workspace,
        check=True,
    )
    subprocess.run(["git", "add", "."], cwd=workspace, check=True)
    subprocess.run(
        ["git", "commit", "-qm", "fixture"], cwd=workspace, check=True
    )
    return digest(_git(workspace, "ls-tree", "-r", "HEAD"))


def apply_setup(task: BenchmarkTask, workspace: pathlib.Path) -> str | None:
    dirty_path = task.setup.get("dirty_path")
    if not isinstance(dirty_path, str):
        return None
    target = workspace / dirty_path
    target.write_text(str(task.setup["dirty_content"]), encoding="utf-8")
    return digest(_git(workspace, "diff", "--binary", "HEAD", "--", dirty_path))


def changed_paths(workspace: pathlib.Path) -> list[str]:
    tracked = _git(workspace, "diff", "--name-only", "-z", "HEAD").split(b"\0")
    untracked = _git(
        workspace, "ls-files", "--others", "--exclude-standard", "-z"
    ).split(b"\0")
    return sorted(
        {
            path.decode("utf-8", errors="strict")
            for path in [*tracked, *untracked]
            if path
        }
    )


def path_matches(path: str, declared: str) -> bool:
    return path == declared or (
        declared.endswith("/") and path.startswith(declared)
    )


def _workspace_evidence(
    task: BenchmarkTask,
    workspace: pathlib.Path,
    base_tree_digest: str,
    dirty_patch_digest: str | None,
) -> dict[str, object]:
    changed = changed_paths(workspace)
    untracked = _git(
        workspace, "ls-files", "--others", "--exclude-standard", "-z"
    ).split(b"\0")
    untracked_paths = [path.decode("utf-8") for path in untracked if path]
    if untracked_paths:
        subprocess.run(
            ["git", "add", "-N", "--", *untracked_paths],
            cwd=workspace,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    diff_bytes = _git(workspace, "diff", "--binary", "HEAD", "--")
    diff_text = diff_bytes.decode("utf-8", errors="replace")

    forbidden_unchanged = all(
        subprocess.run(
            ["git", "diff", "--quiet", "HEAD", "--", declared],
            cwd=workspace,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        == 0
        for declared in task.forbidden_paths
    )
    dirty_path = task.setup.get("dirty_path")
    required_covered = all(
        any(path_matches(path, declared) for path in changed)
        for declared in task.required_changed_paths
    )
    allowed_scope = all(
        path == dirty_path
        or any(path_matches(path, declared) for declared in task.required_changed_paths)
        for path in changed
    )
    current_dirty = None
    if isinstance(dirty_path, str):
        current_dirty = digest(
            _git(workspace, "diff", "--binary", "HEAD", "--", dirty_path)
        )
    return {
        "base_tree_digest": base_tree_digest,
        "diff": diff_text,
        "diff_digest": digest(diff_bytes),
        "changed_files": changed,
        "forbidden_paths_unchanged": forbidden_unchanged,
        "required_scope_satisfied": required_covered and allowed_scope,
        "dirty_patch_digest": dirty_patch_digest,
        "dirty_patch_preserved": current_dirty == dirty_patch_digest,
    }


def _overlay_hidden_acceptance(
    task: BenchmarkTask, workspace: pathlib.Path, root: pathlib.Path
) -> None:
    hidden = root / "tests/coding/acceptance" / task.id
    if not hidden.is_dir():
        return
    for source in hidden.rglob("*"):
        if source.is_file():
            destination = workspace / source.relative_to(hidden)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)


def rewrite_acceptance_command(argv: tuple[str, ...], root: pathlib.Path) -> list[str]:
    if argv[0] == "cargo":
        return ["bash", str(root / "scripts/cargo-agent.sh"), *argv[1:]]
    return list(argv)


def default_binary(environ: Mapping[str, str]) -> pathlib.Path:
    target = environ.get("CARGO_TARGET_DIR")
    if target:
        return pathlib.Path(target) / "debug/aletheon"
    cache_root = environ.get("ALETHEON_CARGO_CACHE_ROOT")
    if not cache_root:
        cache_home = environ.get("XDG_CACHE_HOME")
        if cache_home:
            cache_root = str(pathlib.Path(cache_home) / "aletheon-cargo")
        else:
            cache_root = str(
                pathlib.Path(environ.get("HOME", str(pathlib.Path.home())))
                / ".cache/aletheon-cargo"
            )
    return pathlib.Path(cache_root) / "target/debug/aletheon"


def _check_resources(
    task: BenchmarkTask, results: list[Mapping[str, object]]
) -> dict[str, object]:
    checks: list[dict[str, object]] = []
    if "no_descendant_processes" in task.resource_checks:
        passed = all(result["process_group_reaped"] is True for result in results)
        checks.append({"name": "no_descendant_processes", "passed": passed})
    return {"passed": all(check["passed"] for check in checks), "checks": checks}


def _integer_metric(value: object) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    return None


def _parse_executive(execution: Mapping[str, object]) -> tuple[dict[str, object], bool]:
    raw = execution["_stdout_bytes"]
    assert isinstance(raw, bytes)
    try:
        decoded = raw.decode("utf-8", errors="strict")
        parsed = json.loads(decoded)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return {}, False
    if not isinstance(parsed, dict):
        return {}, False
    # Installed ``aletheon exec --output json`` emits the versioned terminal
    # envelope (type/status/metrics), while older diagnostic clients exposed
    # the flattened stop/success fields. Normalize both at this boundary so
    # acceptance remains based on the authoritative terminal snapshot.
    if parsed.get("type") == "terminal" and "stop" not in parsed:
        status = parsed.get("status")
        if status in {"completed", "blocked", "cancelled", "failed"}:
            parsed["stop"] = status
        metrics = parsed.get("metrics")
        if isinstance(metrics, dict):
            for source, target in (
                ("iterations", "iterations"),
                ("tool_calls_made", "tool_calls_made"),
                ("tool_errors", "tool_errors"),
                ("provider_retries", "provider_retries"),
                ("elapsed_ms", "elapsed_ms"),
            ):
                if target not in parsed and source in metrics:
                    parsed[target] = metrics[source]
            if "inference_rounds" not in parsed and "iterations" in metrics:
                parsed["inference_rounds"] = metrics["iterations"]
            if "success" not in parsed and isinstance(metrics.get("completed_normally"), bool):
                parsed["success"] = metrics["completed_normally"]
    return parsed, True


def _observed_stop(executive: Mapping[str, object]) -> str:
    stop = executive.get("stop")
    return str(stop) if stop in {"completed", "blocked", "cancelled", "failed"} else "unavailable"


def observed_terminal(
    task: BenchmarkTask,
    executive: Mapping[str, object],
    execution: Mapping[str, object],
) -> str:
    if execution["timed_out"]:
        return "cancelled"
    stop = _observed_stop(executive)
    limit = task.setup.get("exec_max_turns")
    if (
        task.expected_terminal == "budget_exhausted"
        and stop == "blocked"
        and isinstance(limit, int)
        and executive.get("iterations") == limit
    ):
        return "budget_exhausted"
    if stop == "completed":
        return "verified"
    return stop


def _build_receipt(
    task: BenchmarkTask,
    binary: pathlib.Path,
    binary_digest: str,
    execution_result: Mapping[str, object],
    executive: Mapping[str, object],
    json_valid: bool,
    workspace: dict[str, object],
    acceptance_results: list[Mapping[str, object]],
    resources: dict[str, object],
) -> dict[str, object]:
    operation_id = executive.get("operation_id")
    operation_id = operation_id if isinstance(operation_id, str) else ""
    stop = _observed_stop(executive)
    observed = observed_terminal(task, executive, execution_result)
    execution = _public_command(execution_result)
    execution.update(
        {
            "json_valid": json_valid,
            "terminal_snapshot": stop != "unavailable",
            "reported_success": executive.get("success")
            if isinstance(executive.get("success"), bool)
            else None,
            "infrastructure_error": execution_result.get("_spawn_error"),
        }
    )
    acceptance = [_public_command(result) for result in acceptance_results]
    reported_elapsed = _integer_metric(executive.get("elapsed_ms"))
    evidence: list[dict[str, object]] = []
    if stop != "unavailable":
        evidence.append(
            {
                "operation_id": operation_id,
                "kind": "terminal_snapshot",
                "observed_stop": stop,
            }
        )
    for index, result in enumerate(acceptance):
        evidence.append(
            {
                "operation_id": operation_id,
                "kind": "acceptance_command",
                "command_index": index,
                "exit_code": result["exit_code"],
                "stdout_digest": result["stdout_digest"],
                "stderr_digest": result["stderr_digest"],
            }
        )
    value: dict[str, object] = {
        "schema_version": 2,
        "task_schema_version": task.schema_version,
        "task_id": task.id,
        "category": task.category,
        "binary": {"path": str(binary), "sha256": binary_digest},
        "operation_id": operation_id,
        "observed_stop": stop,
        "observed_terminal": observed,
        "expected_terminal": task.expected_terminal,
        "execution": execution,
        "workspace": workspace,
        "acceptance": acceptance,
        "evidence": evidence,
        "resources": resources,
        "metrics": {
            "iterations": _integer_metric(executive.get("iterations")),
            "tool_calls": _integer_metric(executive.get("tool_calls_made")),
            "tool_errors": _integer_metric(executive.get("tool_errors")),
            "inference_rounds": _integer_metric(executive.get("inference_rounds")),
            "provider_retries": _integer_metric(executive.get("provider_retries")),
            "active_context_tokens": _integer_metric(
                executive.get("active_context_tokens")
            ),
            "elapsed_ms": reported_elapsed
            if reported_elapsed is not None
            else _integer_metric(execution_result.get("elapsed_ms")),
        },
        "failure": {"class": "none", "reasons": []},
        "verification": {"passed": False},
    }
    failure_class, reasons = classify_failure(value)
    value["failure"] = {"class": failure_class, "reasons": reasons}
    value["verification"] = {"passed": failure_class == "none"}
    return seal(value)


def run_task(
    task_path: pathlib.Path,
    output: pathlib.Path,
    *,
    root: pathlib.Path = ROOT,
    environ: Mapping[str, str] | None = None,
) -> dict[str, object]:
    task = load_task(task_path.resolve(), root)
    environment = dict(os.environ if environ is None else environ)
    binary = pathlib.Path(
        environment.get("ALETHEON_BIN", str(default_binary(environment)))
    ).resolve()
    binary_available = binary.is_file() and os.access(binary, os.X_OK)
    binary_digest = digest(binary.read_bytes()) if binary_available else digest(b"")
    sandbox = environment.get("ALETHEON_CODING_SANDBOX", "auto")
    if sandbox not in {"auto", "require", "forbid"}:
        raise ValueError(f"invalid ALETHEON_CODING_SANDBOX: {sandbox}")

    with tempfile.TemporaryDirectory(prefix=f"aletheon-coding-{task.id}-") as directory:
        temporary_root = pathlib.Path(directory)
        workspace = temporary_root / "workspace"
        fixture = root / "tests/coding/fixtures" / task.fixture
        shutil.copytree(fixture, workspace)
        base_tree_digest = _initialize_workspace(workspace)
        dirty_patch_digest = apply_setup(task, workspace)

        home = temporary_root / "home"
        runtime_dir = temporary_root / "run"
        config = temporary_root / "config"
        home.mkdir()
        runtime_dir.mkdir()
        config.mkdir()
        original_home = pathlib.Path(
            environment.get("HOME", str(pathlib.Path.home()))
        )
        environment.setdefault("RUSTUP_HOME", str(original_home / ".rustup"))
        environment.setdefault("CARGO_HOME", str(original_home / ".cargo"))
        environment.update(
            {
                "HOME": str(home),
                "XDG_RUNTIME_DIR": str(runtime_dir),
                "XDG_CONFIG_HOME": str(config),
            }
        )
        command = [
            str(binary),
            "--cd",
            str(workspace),
            "exec",
            "--prompt",
            task.prompt,
            "--sandbox",
            sandbox,
            "--output",
            "json",
        ]
        max_turns = task.setup.get("exec_max_turns")
        if isinstance(max_turns, int):
            command.extend(["--max-turns", str(max_turns)])

        started = time.monotonic()
        if binary_available:
            execution_result = run_bounded(
                command, workspace, environment, task.timeout_secs
            )
        else:
            execution_result = _deadline_result(command)
            execution_result["timed_out"] = False
            execution_result["stderr"] = f"binary unavailable: {binary}"
            encoded = str(execution_result["stderr"]).encode()
            execution_result["stderr_digest"] = digest(encoded)
            execution_result["_stderr_bytes"] = encoded
            execution_result["_spawn_error"] = "binary_unavailable"
        executive, json_valid = _parse_executive(execution_result)

        workspace_evidence = _workspace_evidence(
            task, workspace, base_tree_digest, dirty_patch_digest
        )
        _overlay_hidden_acceptance(task, workspace, root)
        acceptance_results: list[Mapping[str, object]] = []
        for raw in task.acceptance_commands:
            argv = rewrite_acceptance_command(raw, root)
            remaining = task.timeout_secs - (time.monotonic() - started)
            result = (
                run_bounded(argv, workspace, environment, remaining)
                if remaining > 0
                else _deadline_result(argv)
            )
            acceptance_results.append(result)
            if result["timed_out"]:
                break
        resources = _check_resources(
            task, [execution_result, *acceptance_results]
        )
        value = _build_receipt(
            task,
            binary,
            binary_digest,
            execution_result,
            executive,
            json_valid,
            workspace_evidence,
            acceptance_results,
            resources,
        )
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        return value


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("task", type=pathlib.Path)
    parser.add_argument("--receipt", type=pathlib.Path)
    arguments = parser.parse_args()
    task = load_task(arguments.task.resolve(), ROOT)
    output = arguments.receipt or ROOT / "tests/coding/receipts" / f"{task.id}.json"
    value = run_task(arguments.task, output)
    print(output)
    raise SystemExit(0 if value["verification"]["passed"] else 1)


if __name__ == "__main__":
    main()
