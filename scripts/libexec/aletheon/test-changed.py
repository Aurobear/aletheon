#!/usr/bin/env python3
"""Select and run the narrowest deterministic validation for a Git diff."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time
from typing import Iterable


@dataclass(frozen=True)
class Step:
    kind: str
    command: tuple[str, ...]
    reason: str


def git_lines(root: Path, *args: str, check: bool = True) -> list[str]:
    result = subprocess.run(
        ["git", *args], cwd=root, text=True, capture_output=True, check=False
    )
    if check and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"git {' '.join(args)} failed")
    if result.returncode != 0:
        return []
    return [line for line in result.stdout.splitlines() if line]


def default_base(root: Path) -> str | None:
    configured = os.environ.get("ALETHEON_TEST_BASE")
    if configured:
        return configured
    if git_lines(root, "rev-parse", "--verify", "origin/dev", check=False):
        return "origin/dev"
    if git_lines(root, "rev-parse", "--verify", "HEAD^", check=False):
        return "HEAD^"
    return None


def changed_paths(root: Path, base: str | None) -> list[str]:
    paths: set[str] = set()
    if base:
        merge_base = git_lines(root, "merge-base", base, "HEAD")
        if merge_base:
            paths.update(
                git_lines(
                    root,
                    "diff",
                    "--name-only",
                    "--diff-filter=ACMR",
                    f"{merge_base[0]}..HEAD",
                )
            )
    paths.update(git_lines(root, "diff", "--name-only", "--diff-filter=ACMR"))
    paths.update(
        git_lines(
            root, "diff", "--cached", "--name-only", "--diff-filter=ACMR"
        )
    )
    paths.update(git_lines(root, "ls-files", "--others", "--exclude-standard"))
    return sorted(paths)


def load_metadata(root: Path) -> dict:
    result = subprocess.run(
        [
            "bash",
            "scripts/cargo-agent.sh",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
        ],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "cargo metadata failed")
    return json.loads(result.stdout)


def relative(root: Path, value: str) -> str:
    return Path(value).resolve().relative_to(root.resolve()).as_posix()


def append_unique(steps: list[Step], step: Step) -> None:
    if not any(existing.command == step.command for existing in steps):
        steps.append(step)


def derive_steps(root: Path, paths: Iterable[str], metadata: dict) -> list[Step]:
    changed = set(paths)
    cargo = ("bash", "scripts/cargo-agent.sh")
    packages = metadata.get("packages", [])
    workspace_ids = set(metadata.get("workspace_members", []))
    workspace_packages = [package for package in packages if package["id"] in workspace_ids]
    names = {package["name"] for package in workspace_packages}
    package_roots = {
        package["name"]: Path(relative(root, package["manifest_path"])).parent.as_posix()
        for package in workspace_packages
    }
    affected = {
        name
        for name, package_root in package_roots.items()
        if any(path == package_root or path.startswith(f"{package_root}/") for path in changed)
    }
    steps: list[Step] = []

    if "Cargo.toml" in changed or "Cargo.lock" in changed:
        append_unique(
            steps,
            Step(
                "check",
                cargo + ("check", "--workspace", "--all-targets"),
                "workspace manifest or lockfile changed",
            ),
        )

    for package in sorted(workspace_packages, key=lambda item: item["name"]):
        name = package["name"]
        if name not in affected:
            continue
        package_root = package_roots[name]
        package_paths = {
            path
            for path in changed
            if path == package_root or path.startswith(f"{package_root}/")
        }
        append_unique(
            steps,
            Step("check", cargo + ("check", "-p", name), f"package {name} changed"),
        )
        if any(
            path.endswith(".rs") and f"{package_root}/tests/" not in path
            for path in package_paths
        ) or any(path.endswith("Cargo.toml") for path in package_paths):
            append_unique(
                steps,
                Step(
                    "test",
                    cargo + ("test", "-p", name, "--lib"),
                    f"package {name} library code changed",
                ),
            )

        exact_targets: set[str] = set()
        broad_integration = False
        test_targets = {
            relative(root, target["src_path"]): target["name"]
            for target in package.get("targets", [])
            if "test" in target.get("kind", [])
        }
        for path in package_paths:
            if f"{package_root}/tests/" not in path or not path.endswith(".rs"):
                continue
            target = test_targets.get(path)
            if target:
                exact_targets.add(target)
            else:
                broad_integration = True
        for target in sorted(exact_targets):
            append_unique(
                steps,
                Step(
                    "test",
                    cargo + ("test", "-p", name, "--test", target),
                    f"integration target {name}/{target} changed",
                ),
            )
        if broad_integration:
            append_unique(
                steps,
                Step(
                    "test",
                    cargo + ("test", "-p", name, "--tests"),
                    f"shared integration support in {name} changed",
                ),
            )

    for package in sorted(workspace_packages, key=lambda item: item["name"]):
        name = package["name"]
        if name in affected:
            continue
        dependencies = {
            dependency["name"]
            for dependency in package.get("dependencies", [])
            if dependency["name"] in names
        }
        impacted = sorted(dependencies & affected)
        if impacted:
            append_unique(
                steps,
                Step(
                    "check",
                    cargo + ("check", "-p", name),
                    f"direct dependency changed: {', '.join(impacted)}",
                ),
            )

    if any(path == "scripts/aletheon.sh" or path.startswith("scripts/lib/aletheon/") for path in changed):
        append_unique(
            steps,
            Step(
                "script",
                ("bash", "tests/suites/operations/cli_static_test.sh"),
                "operations command surface changed",
            ),
        )
    if any(path.startswith("scripts/") for path in changed):
        append_unique(
            steps,
            Step(
                "script",
                ("bash", "tests/suites/operations/script_surface_test.sh"),
                "repository scripts changed",
            ),
        )
    return steps


def print_plan(paths: list[str], steps: list[Step]) -> None:
    print(f"Aletheon changed validation: {len(paths)} changed path(s), {len(steps)} step(s)")
    for step in steps:
        print(f"[{step.kind}] {shlex.join(step.command)}  # {step.reason}")


def report_payload(paths: list[str], steps: list[Step], results: list[dict]) -> dict:
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "changed_paths": paths,
        "selected_steps": [
            {**asdict(step), "command": list(step.command)} for step in steps
        ],
        "results": results,
    }


def write_report(path: Path | None, payload: dict) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def load_report(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def report_commands(payload: dict) -> set[tuple[str, ...]]:
    return {
        tuple(step.get("command", [])) for step in payload.get("selected_steps", [])
    }


def select_report_steps(
    payload: dict, available: list[Step], failed_only: bool
) -> list[Step]:
    available_commands = {step.command for step in available}
    recorded_commands = report_commands(payload)
    if recorded_commands != available_commands:
        raise ValueError("validation report does not match the current diff-derived plan")
    statuses = {
        tuple(result.get("command", [])): result.get("status")
        for result in payload.get("results", [])
    }
    if failed_only:
        return [step for step in available if statuses.get(step.command) == "failed"]
    return [step for step in available if statuses.get(step.command) != "passed"]


def merge_result(results: list[dict], result: dict) -> None:
    command = tuple(result["command"])
    results[:] = [item for item in results if tuple(item.get("command", [])) != command]
    results.append(result)


def execute(
    root: Path,
    paths: list[str],
    steps: list[Step],
    selected_plan: list[Step],
    keep_going: bool,
    report_path: Path | None,
    initial_results: list[dict],
) -> int:
    failures = 0
    results = list(initial_results)
    for index, step in enumerate(steps, 1):
        started = time.monotonic()
        print(f"\n[{index}/{len(steps)}] {shlex.join(step.command)}", flush=True)
        result = subprocess.run(step.command, cwd=root, check=False)
        elapsed = time.monotonic() - started
        status = "passed" if result.returncode == 0 else f"failed ({result.returncode})"
        print(f"[{step.kind}] {status} in {elapsed:.2f}s", flush=True)
        merge_result(
            results,
            {
                "kind": step.kind,
                "command": list(step.command),
                "reason": step.reason,
                "status": "passed" if result.returncode == 0 else "failed",
                "exit_code": result.returncode,
                "duration_ms": round(elapsed * 1000),
            },
        )
        write_report(report_path, report_payload(paths, selected_plan, results))
        if result.returncode != 0:
            failures += 1
            if not keep_going:
                return result.returncode
    return 1 if failures else 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="compare committed changes with this Git revision")
    parser.add_argument("--plan", action="store_true", help="print the selected steps without running them")
    parser.add_argument("--keep-going", action="store_true", help="continue after a failed validation step")
    parser.add_argument("--report", type=Path, help="write a machine-readable JSON validation report")
    parser.add_argument(
        "--rerun-failed",
        type=Path,
        metavar="REPORT",
        help="run only failed steps that remain in the current validation plan",
    )
    parser.add_argument(
        "--resume",
        type=Path,
        metavar="REPORT",
        help="rerun failed and not-yet-run steps while preserving passed receipts",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parents[3]
    paths = changed_paths(root, args.base or default_base(root))
    if not paths:
        print("Aletheon changed validation: no changed paths")
        return 0
    if args.rerun_failed and args.resume:
        raise ValueError("--rerun-failed and --resume are mutually exclusive")
    selected_plan = derive_steps(root, paths, load_metadata(root))
    steps = selected_plan
    prior_results: list[dict] = []
    prior_report = args.rerun_failed or args.resume
    if prior_report:
        payload = load_report(prior_report)
        prior_results = payload.get("results", [])
        steps = select_report_steps(payload, selected_plan, bool(args.rerun_failed))
    report_path = args.report or prior_report
    print_plan(paths, steps)
    if args.plan or not steps:
        write_report(report_path, report_payload(paths, selected_plan, prior_results))
        return 0
    return execute(
        root,
        paths,
        steps,
        selected_plan,
        args.keep_going,
        report_path,
        prior_results,
    )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, ValueError, OSError) as error:
        print(f"aletheon test changed: {error}", file=sys.stderr)
        raise SystemExit(2)
