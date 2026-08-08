#!/usr/bin/env python3
"""Compile mapped Rust test binaries together, then run only mapped tests."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib


SAFE_NAME = re.compile(r"^[A-Za-z0-9_-]+$")
SAFE_FILTER = re.compile(r"^[A-Za-z0-9_:.-]+$")
RUNNING_TESTS = re.compile(r"^running ([0-9]+) tests?$", re.MULTILINE)


def load_selections(root: Path, packages: list[str]) -> dict[str, dict]:
    with (root / ".aletheon-validation.toml").open("rb") as handle:
        payload = tomllib.load(handle)
    raw_groups = payload.get("changed_validation", {}).get("rust_test_groups", [])
    groups = {group.get("package"): group for group in raw_groups}
    missing = sorted(set(packages) - groups.keys())
    if missing:
        raise ValueError("packages lack mapped test groups: " + ", ".join(missing))
    selections: dict[str, dict] = {}
    for package in packages:
        group = groups[package]
        filters = group.get("lib_filters", [])
        targets = group.get("integration_targets", [])
        if not all(isinstance(value, str) and SAFE_FILTER.fullmatch(value) for value in filters):
            raise ValueError(f"invalid lib filter in test group {package}")
        if not all(isinstance(value, str) and SAFE_NAME.fullmatch(value) for value in targets):
            raise ValueError(f"invalid integration target in test group {package}")
        selections[package] = {
            "lib_filters": sorted(set(filters)),
            "integration_targets": sorted(set(targets)),
        }
    return selections


def compile_binaries(
    root: Path,
    packages: list[str],
    target_args: list[str],
    expected_targets: set[str],
) -> dict[str, Path]:
    command = ["bash", "scripts/cargo-agent.sh", "test"]
    for package in packages:
        command += ["-p", package]
    command += target_args + ["--no-run", "--message-format=json"]
    process = subprocess.Popen(
        command,
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
    )
    assert process.stdout is not None
    executables: dict[str, Path] = {}
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            print(line, end="", file=sys.stderr)
            continue
        if message.get("reason") == "compiler-message":
            rendered = message.get("message", {}).get("rendered")
            if rendered:
                print(rendered, end="", file=sys.stderr)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        target = message.get("target", {})
        name = target.get("name")
        if name in expected_targets:
            executables[name] = Path(message["executable"])
    process.stdout.close()
    returncode = process.wait()
    if returncode != 0:
        raise RuntimeError(f"mapped test compilation failed ({returncode})")
    missing = sorted(expected_targets - executables.keys())
    if missing:
        raise RuntimeError("Cargo did not emit mapped test binaries: " + ", ".join(missing))
    return executables


def run_checked(root: Path, command: list[str], selection: str) -> int:
    process = subprocess.Popen(
        command,
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    assert process.stdout is not None
    output: list[str] = []
    for line in process.stdout:
        print(line, end="", flush=True)
        output.append(line)
    process.stdout.close()
    returncode = process.wait()
    if returncode != 0:
        return returncode
    selected = sum(int(match) for match in RUNNING_TESTS.findall("".join(output)))
    if selected == 0:
        print(
            f"aletheon filtered test: {selection} matched zero tests",
            file=sys.stderr,
        )
        return 3
    print(f"aletheon filtered test: {selection} selected {selected} test(s)")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", action="append", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    packages = sorted(set(args.package))
    if not all(SAFE_NAME.fullmatch(package) for package in packages):
        raise ValueError("package contains unsupported characters")

    root = Path(__file__).resolve().parents[3]
    selections = load_selections(root, packages)
    lib_packages = [
        package for package in packages if selections[package]["lib_filters"]
    ]
    if lib_packages:
        lib_targets = {package.replace("-", "_") for package in lib_packages}
        lib_binaries = compile_binaries(
            root, lib_packages, ["--lib"], lib_targets
        )
        for package in lib_packages:
            executable = lib_binaries[package.replace("-", "_")]
            for test_filter in selections[package]["lib_filters"]:
                result = run_checked(
                    root,
                    [str(executable), test_filter, "--quiet"],
                    f"{package} --lib {test_filter}",
                )
                if result != 0:
                    return result

    integration_packages = [
        package for package in packages if selections[package]["integration_targets"]
    ]
    integration_targets = {
        target
        for package in integration_packages
        for target in selections[package]["integration_targets"]
    }
    if len(integration_targets) != sum(
        len(selections[package]["integration_targets"])
        for package in integration_packages
    ):
        raise ValueError("mapped integration target names must be unique across packages")
    if integration_targets:
        target_args = [
            argument
            for target in sorted(integration_targets)
            for argument in ("--test", target)
        ]
        binaries = compile_binaries(
            root, integration_packages, target_args, integration_targets
        )
        for target in sorted(integration_targets):
            result = run_checked(
                root, [str(binaries[target]), "--quiet"], f"--test {target}"
            )
            if result != 0:
                return result
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"aletheon filtered test: {error}", file=sys.stderr)
        raise SystemExit(2)
