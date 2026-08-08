"""Command-line entry point for Aletheon Nightwatch."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .config import load_settings
from .scheduler import Nightwatch


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(prog="aletheon-lab")
    value.add_argument("--config", type=Path, required=True)
    subcommands = value.add_subparsers(dest="command", required=True)
    once = subcommands.add_parser("run", help="run one reviewed campaign")
    once.add_argument("--case", required=True)
    cycle = subcommands.add_parser("cycle", help="run every enabled due campaign")
    cycle.add_argument("--force", action="store_true")
    watch = subcommands.add_parser("watch", help="run the continuous scheduler")
    watch.add_argument("--max-cycles", type=int)
    return value


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    try:
        settings = load_settings(arguments.config.resolve())
        nightwatch = Nightwatch(settings, output=sys.stdout)
        try:
            if arguments.command == "run":
                result = nightwatch.run_case(arguments.case)
                print(json.dumps(result, sort_keys=True, ensure_ascii=False))
                return 0 if result["outcome"] == "passed" else 1
            if arguments.command == "cycle":
                results = nightwatch.cycle(force=arguments.force)
                return (
                    0
                    if nightwatch.last_cycle_error_count == 0
                    and all(item["outcome"] == "passed" for item in results)
                    else 1
                )
            nightwatch.watch(max_cycles=arguments.max_cycles)
            return 0
        finally:
            nightwatch.close()
    except (OSError, TypeError, ValueError, RuntimeError, KeyError) as error:
        print(f"aletheon-lab: {type(error).__name__}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
