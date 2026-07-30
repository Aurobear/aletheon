import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "release_metadata.py"
SPEC = importlib.util.spec_from_file_location("release_metadata", SCRIPT)
assert SPEC and SPEC.loader
release_metadata = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_metadata)


class ReleaseMetadataTests(unittest.TestCase):
    def test_metadata_is_deterministic_and_records_binary_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.lock").write_text(
                '''version = 4

[[package]]
name = "example"
version = "1.2.3"
source = "registry+https://example.invalid/index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
''',
                encoding="utf-8",
            )
            binary = root / "aletheon"
            binary.write_bytes(b"release-binary")
            environment = {
                "GITHUB_SHA": "0123456789abcdef",
                "SOURCE_DATE_EPOCH": "1700000000",
            }
            with mock.patch.dict(os.environ, environment, clear=False):
                first = release_metadata.generate(root, binary, "x86_64-unknown-linux-gnu")
                second = release_metadata.generate(root, binary, "x86_64-unknown-linux-gnu")

            self.assertEqual(first, second)
            annotation = json.loads(first["annotations"][0]["comment"])
            self.assertEqual(annotation["binary_sha256"], release_metadata.sha256(binary))
            self.assertEqual(annotation["git_revision"], environment["GITHUB_SHA"])
            self.assertEqual(first["spdxVersion"], "SPDX-2.3")
            self.assertEqual(len(first["packages"]), 1)


if __name__ == "__main__":
    unittest.main()
