#!/usr/bin/env python3
"""Deterministic E1 suite manifest generator.

Usage:
  python3 generate_e1_suite.py          # regenerate manifest from source
  python3 generate_e1_suite.py --check  # verify manifest matches source (no write)

The manifest is a deterministic projection of:
- The catalog (task metadata including key semantics)
- All public workspace files (sorted file-path→SHA-256 maps)
- All hidden oracle files (sorted file-path→SHA-256 maps for solution dirs, single digest for oracles)
- All hidden solution files (sorted file-path→SHA-256 maps)

Normal mode writes canonical manifest bytes. --check mode recomputes
and byte-compares without writing. Drift in any source file is detected.

Rejects missing assets, symlinks, special files, and path-escape patterns.
"""
import argparse
import hashlib
import json
import sys
import tomllib
from pathlib import Path

BASE = Path(__file__).resolve().parent

# Import authoritative catalog validation from runner
sys.path.insert(0, str(BASE))
import runner as _runner

# Semantic catalog fields that MUST bind the manifest (any change = new digest)
CATALOG_SEMANTIC_FIELDS = [
    "id", "category", "provenance", "license", "source_kind",
    "network", "permission", "risk", "timeout", "cpu_seconds",
    "memory_mb", "max_open_files", "max_processes",
    "injected_failures", "expected_failure_class", "task_prompt",
    "allowed_paths", "forbidden_paths", "required_paths",
    "oracle_path", "solution_path", "fixture_path",
]


def _file_digest(path: Path) -> str:
    """SHA-256 hex digest of file contents."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _validate_and_hash_files(root: Path) -> dict:
    """Return sorted {relpath: sha256} for all regular files under root.
    Rejects symlinks, special files, and path-escape patterns."""
    result = {}
    if not root.exists():
        raise FileNotFoundError(f"directory not found: {root}")
    if root.is_symlink():
        raise ValueError(f"symlink not allowed: {root}")
    if not root.is_dir():
        raise ValueError(f"not a directory: {root}")
    for f in sorted(root.rglob("*")):
        # Reject symlinks
        if f.is_symlink():
            raise ValueError(f"symlink not allowed: {f.relative_to(root)}")
        # Reject special files (not regular file and not directory)
        if not f.is_file() and not f.is_dir():
            raise ValueError(f"special file not allowed: {f.relative_to(root)}")
        if f.is_file():
            rel = str(f.relative_to(root))
            # Reject path-escape
            if ".." in rel.split("/"):
                raise ValueError(f"path escape: {rel}")
            result[rel] = _file_digest(f)
    return result


def compute_manifest() -> dict:
    """Build canonical manifest from current source."""
    # Load catalog
    catalog_path = BASE / "catalog.toml"
    with open(catalog_path, "rb") as f:
        catalog = tomllib.load(f)

    tasks = catalog.get("tasks", [])

    # Use the same authoritative catalog validation as the runner
    catalog_errors = _runner._validate_catalog(tasks, BASE)
    if catalog_errors:
        for e in catalog_errors:
            print(f"CATALOG_ERROR: {e}", file=sys.stderr)
        raise SystemExit(1)

    if len(tasks) != 30:
        raise ValueError(f"catalog: expected 30 tasks, found {len(tasks)}")

    # Hash all public workspaces (sorted file→digest map)
    workspace_maps = {}
    for t in tasks:
        wp = BASE / t["fixture_path"]
        workspace_maps[t["id"]] = _validate_and_hash_files(wp)

    # Hash all hidden oracles
    oracle_digests = {}
    for t in tasks:
        op = BASE / t["oracle_path"]
        if not op.exists():
            raise FileNotFoundError(f"oracle missing: {t['oracle_path']}")
        if op.is_symlink():
            raise ValueError(f"oracle symlink: {t['oracle_path']}")
        if not op.is_file():
            raise ValueError(f"oracle not a file: {t['oracle_path']}")
        oracle_digests[t["id"]] = _file_digest(op)

    # Hash all hidden solutions (sorted file→digest map)
    solution_maps = {}
    for t in tasks:
        sp = BASE / t["solution_path"]
        solution_maps[t["id"]] = _validate_and_hash_files(sp)

    # Compute catalog canonical SHA (semantic fields only)
    catalog_semantic = []
    for t in tasks:
        entry = {}
        for key in CATALOG_SEMANTIC_FIELDS:
            entry[key] = t.get(key)
        catalog_semantic.append(entry)
    catalog_sha = hashlib.sha256(
        json.dumps(catalog_semantic, sort_keys=True).encode()
    ).hexdigest()

    manifest = {
        "schema_version": 2,
        "suite": "E1",
        "suite_version": catalog.get("suite_version", "3.0.0"),
        "generator": "tests/robot_engineering/generate_e1_suite.py",
        "catalog_sha256": catalog_sha,
        "task_count": len(tasks),
        "category_counts": {
            cat: len([t for t in tasks if t["category"] == cat])
            for cat in sorted(set(t["category"] for t in tasks))
        },
        "failure_classes": sorted(set(
            fc for t in tasks for fc in t.get("injected_failures", [])
        )),
        "tasks": [
            {
                "id": t["id"],
                "category": t["category"],
                "expected_failure_class": t.get("expected_failure_class", "unknown"),
                "injected_failures": t.get("injected_failures", []),
                "risk": t.get("risk", "unknown"),
                "permission": t.get("permission", "unknown"),
                "timeout": t.get("timeout", 0),
                "cpu_seconds": t.get("cpu_seconds", 0),
                "memory_mb": t.get("memory_mb", 0),
                "network": t.get("network", "deny-all"),
                "source_kind": t.get("source_kind", "python"),
                "provenance": t.get("provenance", ""),
                "license": t.get("license", ""),
                "fixture_path": t.get("fixture_path", ""),
                "oracle_path": t.get("oracle_path", ""),
                "solution_path": t.get("solution_path", ""),
                "allowed_paths": t.get("allowed_paths", []),
                "forbidden_paths": t.get("forbidden_paths", []),
                "required_paths": t.get("required_paths", []),
                # Aggregate workspace digest (derived from sorted file map)
                "workspace_digest": hashlib.sha256(
                    json.dumps(
                        workspace_maps.get(t["id"], {}), sort_keys=True
                    ).encode()
                ).hexdigest(),
                # Individual oracle file digest
                "oracle_digest": oracle_digests.get(t["id"], ""),
                # Aggregate solution digest (derived from sorted file map)
                "solution_digest": hashlib.sha256(
                    json.dumps(
                        solution_maps.get(t["id"], {}), sort_keys=True
                    ).encode()
                ).hexdigest(),
                # Full sorted file→SHA-256 maps
                "workspace_file_map": workspace_maps.get(t["id"], {}),
                "solution_file_map": solution_maps.get(t["id"], {}),
            }
            for t in tasks
        ],
    }

    # Compute canonical digest of the manifest (without canonical_digest field)
    manifest_bytes = json.dumps(manifest, indent=2, sort_keys=True).encode()
    manifest["canonical_digest"] = hashlib.sha256(manifest_bytes).hexdigest()

    return manifest


def write_manifest(manifest: dict) -> None:
    """Write manifest to disk."""
    path = BASE / "manifest.json"
    text = json.dumps(manifest, indent=2, sort_keys=True)
    path.write_text(text + "\n", encoding="utf-8")


def check_manifest() -> bool:
    """Verify manifest on disk matches source projection. Never writes."""
    manifest_path = BASE / "manifest.json"
    if not manifest_path.is_file():
        print("ERROR: manifest.json not found", file=sys.stderr)
        return False

    current_bytes = manifest_path.read_bytes()

    # Compute fresh projection
    fresh = compute_manifest()
    fresh_bytes = json.dumps(fresh, indent=2, sort_keys=True).encode()
    fresh_bytes += b"\n"

    if current_bytes != fresh_bytes:
        print("ERROR: manifest drift detected", file=sys.stderr)
        # Show diff hints
        try:
            current = json.loads(current_bytes.decode())
            if current.get("canonical_digest") != fresh.get("canonical_digest"):
                print(f"  canonical_digest: {current.get('canonical_digest', 'N/A')[:16]}... vs {fresh.get('canonical_digest', 'N/A')[:16]}...", file=sys.stderr)
            if current.get("task_count") != fresh.get("task_count"):
                print(f"  task_count: {current.get('task_count')} vs {fresh.get('task_count')}", file=sys.stderr)
        except Exception:
            pass
        return False

    print(f"OK: manifest digest {fresh['canonical_digest'][:16]}... matches source")
    return True


def main():
    ap = argparse.ArgumentParser(description="E1 Suite Manifest Generator")
    ap.add_argument("--check", action="store_true",
                    help="Verify manifest matches source (never overwrites)")
    ap.add_argument("--root", default=None,
                    help="Override BASE directory (for testing)")
    args = ap.parse_args()

    global BASE
    if args.root:
        BASE = Path(args.root).resolve()

    if args.check:
        ok = check_manifest()
        sys.exit(0 if ok else 1)
    else:
        manifest = compute_manifest()
        write_manifest(manifest)
        print(f"Manifest written: {manifest['canonical_digest'][:16]}...")
        ok = check_manifest()
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
