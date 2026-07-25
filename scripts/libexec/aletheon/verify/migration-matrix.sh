#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd -P)
matrix=${ALETHEON_MIGRATION_MATRIX:-"$repo_root/config/release/migration-matrix.toml"}
[[ -f "$matrix" && ! -L "$matrix" ]] || { echo "migration matrix missing or unsafe: $matrix" >&2; exit 1; }

python3 - "$repo_root" "$matrix" <<'PY'
from __future__ import annotations
import pathlib, re, sys, tomllib
root = pathlib.Path(sys.argv[1]).resolve()
path = pathlib.Path(sys.argv[2]).resolve()
with path.open("rb") as stream:
    matrix = tomllib.load(stream)
if matrix.get("format_version") != 1:
    raise SystemExit("migration matrix: unsupported format_version")
if matrix.get("mixed_version_operation") != "forbidden":
    raise SystemExit("migration matrix: mixed-version operation must be forbidden")
if matrix.get("binary_only_rollback_after_data_change") != "forbidden":
    raise SystemExit("migration matrix: binary-only rollback must be forbidden")
rows = matrix.get("transition", [])
required = {"event_spine", "session", "memory", "agent", "agora", "dasein", "config"}
seen = set()
for index, row in enumerate(rows, 1):
    label = f"transition {index}"
    component = row.get("component")
    if component in seen:
        raise SystemExit(f"migration matrix: duplicate component {component}")
    seen.add(component)
    for key in ("from", "to", "kind", "backup_required", "forward", "rollback", "integrity_query"):
        if key not in row or row[key] == "":
            raise SystemExit(f"migration matrix: {label} missing {key}")
    if row["backup_required"] is not True:
        raise SystemExit(f"migration matrix: {component} must require backup")
    if row.get("data_change") and row["rollback"] != "restore_matching_data_and_binary":
        raise SystemExit(f"migration matrix: {component} data change permits binary-only rollback")
    if row["kind"] == "migration":
        for key in ("fixture", "reopen_test"):
            if not row.get(key):
                raise SystemExit(f"migration matrix: {component} migration missing {key}")
        fixture = (root / row["fixture"]).resolve()
        if root not in fixture.parents or not fixture.is_file():
            raise SystemExit(f"migration matrix: unsafe or missing fixture {row['fixture']}")
        source = fixture.read_text(encoding="utf-8")
        if row["reopen_test"] not in source:
            raise SystemExit(f"migration matrix: reopen test {row['reopen_test']} not found")
        if row["integrity_query"] != "PRAGMA integrity_check":
            raise SystemExit(f"migration matrix: {component} SQLite migration lacks integrity query")
    elif row["kind"] == "contract":
        evidence = (root / row.get("evidence", "")).resolve()
        if root not in evidence.parents or not evidence.is_file():
            raise SystemExit(f"migration matrix: unsafe or missing evidence for {component}")
        if row["from"] != row["to"]:
            raise SystemExit(f"migration matrix: non-migration {component} changes version")
    else:
        raise SystemExit(f"migration matrix: unknown kind for {component}")
missing = required - seen
extra = seen - required
if missing or extra:
    raise SystemExit(f"migration matrix: component mismatch missing={sorted(missing)} extra={sorted(extra)}")
print(f"migration matrix verified: {len(rows)} components; binary-only rollback forbidden")
PY

# Execute the declared pre-migration/reopen evidence. Production installed-host
# tests execute the declared PRAGMA integrity query against every resulting DB.
CARGO_INCREMENTAL=0 bash "$repo_root/scripts/cargo-agent.sh" test -p mnemosyne --test gbrain_spool \
  legacy_migration_redacts_commits_then_renames_and_restarts_idempotently -- --exact
CARGO_INCREMENTAL=0 bash "$repo_root/scripts/cargo-agent.sh" test -p executive --test agent_control_repository \
  repository_migrates_pre_workspace_rows_without_losing_runs -- --exact
