#!/usr/bin/env python3
"""Generate deterministic SPDX 2.3 release metadata from Cargo.lock."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tomllib
from datetime import datetime, timezone
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repository_revision(root: Path) -> str:
    configured = os.environ.get("GITHUB_SHA")
    if configured:
        return configured
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True
    ).strip()


def creation_time(root: Path) -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch is None:
        epoch = subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "HEAD"], cwd=root, text=True
        ).strip()
    return datetime.fromtimestamp(int(epoch), timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def spdx_id(name: str, version: str, index: int) -> str:
    safe = "".join(character if character.isalnum() else "-" for character in name)
    return f"SPDXRef-Package-{safe}-{version}-{index}"


def generate(root: Path, binary: Path, target: str) -> dict[str, object]:
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    revision = repository_revision(root)
    packages = []
    package_ids = []
    for index, package in enumerate(lock["package"]):
        identifier = spdx_id(package["name"], package["version"], index)
        package_ids.append(identifier)
        entry: dict[str, object] = {
            "SPDXID": identifier,
            "name": package["name"],
            "versionInfo": package["version"],
            "downloadLocation": package.get("source", "NOASSERTION"),
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }
        if checksum := package.get("checksum"):
            entry["checksums"] = [{"algorithm": "SHA256", "checksumValue": checksum}]
        packages.append(entry)

    binary_digest = sha256(binary)
    namespace = (
        "https://github.com/Aurobear/aletheon/releases/"
        f"{revision}/{target}/{binary_digest}"
    )
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"aletheon-{target}",
        "documentNamespace": namespace,
        "creationInfo": {
            "created": creation_time(root),
            "creators": ["Tool: scripts/release_metadata.py"],
        },
        "documentDescribes": package_ids,
        "packages": packages,
        "externalDocumentRefs": [],
        "annotations": [
            {
                "annotationType": "OTHER",
                "annotator": "Tool: scripts/release_metadata.py",
                "annotationDate": creation_time(root),
                "comment": json.dumps(
                    {
                        "binary": binary.name,
                        "binary_sha256": binary_digest,
                        "git_revision": revision,
                        "target": target,
                    },
                    sort_keys=True,
                ),
            }
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    metadata = generate(args.root.resolve(), args.binary.resolve(), args.target)
    args.output.write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
