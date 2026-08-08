"""Tests for acceptance contract validation helpers."""

import copy
import hashlib
from datetime import datetime, timezone
import unittest

from harness.acceptance_contract import (
    ContractError,
    canonical_bytes,
    sha256_json,
    validate_provenance,
    validate_waiver,
)

VALID_PROVENANCE = {
    "repo": {"sha": "a" * 40, "dirty": False},
    "build": {"profile": "release", "features": ["fast", "safe"]},
    "environment_level": "installed",
    "installed_artifact": {"path": "/usr/bin/aletheon", "sha256": "b" * 64},
    "client": {"version": "1.2.3", "protocol_version": "1"},
    "daemons": {
        "machine": {
            "path": "/usr/bin/aletheon",
            "sha256": "b" * 64,
            "version": "1.2.3",
            "protocol_version": "1",
        },
        "user": {
            "path": "/usr/bin/aletheon",
            "sha256": "b" * 64,
            "version": "1.2.3",
            "protocol_version": "1",
        },
    },
    "provider": {"provider_id": "p", "model_id": "m", "endpoint_id": "localhost"},
    "fixture_digest": "e" * 64,
    "generation_id": "gen-1",
}

VALID_WAIVER = {"approver": "alice", "reason": "ok", "expires_at": "2026-01-01T00:00:00Z"}

REFERENCE_TIME = datetime(2025, 1, 1, tzinfo=timezone.utc)


def _del_path(obj, path):
    parts = path.split(".")
    for part in parts[:-1]:
        obj = obj[part]
    del obj[parts[-1]]


def _set_path(obj, path, value):
    parts = path.split(".")
    for part in parts[:-1]:
        obj = obj[part]
    obj[parts[-1]] = value


def make_input():
    return copy.deepcopy(VALID_PROVENANCE)


class AcceptanceContractTest(unittest.TestCase):
    def assert_provenance_error(self, mutator):
        data = make_input()
        mutator(data)
        with self.assertRaises(ContractError):
            validate_provenance(data)

    def test_valid_installed_provenance_is_normalized_copy(self):
        data = make_input()
        original = copy.deepcopy(data)
        result = validate_provenance(data)
        self.assertEqual(result, original)
        self.assertIsNot(result, data)
        self.assertIsNot(result["repo"], data["repo"])
        self.assertEqual(result["build"]["features"], ["fast", "safe"])

    def test_provenance_missing_keys(self):
        paths = [
            "repo", "build", "environment_level", "installed_artifact", "client",
            "daemons", "provider", "fixture_digest", "generation_id",
            "repo.sha", "repo.dirty", "build.profile", "build.features",
            "installed_artifact.path", "installed_artifact.sha256",
            "client.version", "client.protocol_version",
            "daemons.machine", "daemons.user",
            "daemons.machine.path", "daemons.machine.sha256",
            "daemons.user.path", "daemons.user.sha256",
            "daemons.machine.version", "daemons.machine.protocol_version",
            "daemons.user.version", "daemons.user.protocol_version",
            "provider.provider_id", "provider.model_id", "provider.endpoint_id",
        ]
        for path in paths:
            with self.subTest(missing=path):
                data = make_input()
                _del_path(data, path)
                with self.assertRaises(ContractError):
                    validate_provenance(data)

    def test_provenance_unknown_keys(self):
        paths = [
            "extra", "repo.extra", "build.extra", "installed_artifact.extra",
            "client.extra", "daemons.extra", "daemons.machine.extra",
            "daemons.user.extra", "provider.extra",
        ]
        for path in paths:
            with self.subTest(unknown=path):
                data = make_input()
                _set_path(data, path, "x")
                with self.assertRaises(ContractError):
                    validate_provenance(data)

    def test_provenance_invalid_values(self):
        cases = [
            ("repo.sha bad length", lambda d: d["repo"].update(sha="abc")),
            ("repo.sha uppercase", lambda d: d["repo"].update(sha="A" * 40)),
            ("repo.dirty not bool", lambda d: d["repo"].update(dirty="yes")),
            ("environment_level invalid", lambda d: d.update(environment_level="prod")),
            ("installed_artifact.path wrong", lambda d: d["installed_artifact"].update(path="/bad")),
            ("generation_id invalid char", lambda d: d.update(generation_id="bad gen")),
            ("fixture_digest bad", lambda d: d.update(fixture_digest="xyz")),
            ("installed_artifact.sha256 bad", lambda d: d["installed_artifact"].update(sha256="xyz")),
            ("daemons.machine.sha256 bad", lambda d: d["daemons"]["machine"].update(sha256="xyz")),
            ("build.profile too many UTF-8 bytes", lambda d: d["build"].update(profile="é" * 3000)),
            ("daemons.machine.version empty", lambda d: d["daemons"]["machine"].update(version="")),
            ("daemons.machine.protocol_version empty", lambda d: d["daemons"]["machine"].update(protocol_version="")),
            ("daemons.user.version empty", lambda d: d["daemons"]["user"].update(version="")),
            ("daemons.user.protocol_version empty", lambda d: d["daemons"]["user"].update(protocol_version="")),
            ("installed: daemons.machine.path mismatch", lambda d: d["daemons"]["machine"].update(path="/wrong")),
            ("installed: daemons.machine.sha256 mismatch", lambda d: d["daemons"]["machine"].update(sha256="f" * 64)),
            ("installed: daemons.machine.version mismatch", lambda d: d["daemons"]["machine"].update(version="0.0.0")),
            ("installed: daemons.machine.protocol_version mismatch", lambda d: d["daemons"]["machine"].update(protocol_version="2")),
            ("installed: daemons.user.path mismatch", lambda d: d["daemons"]["user"].update(path="/wrong")),
            ("installed: daemons.user.sha256 mismatch", lambda d: d["daemons"]["user"].update(sha256="f" * 64)),
            ("installed: daemons.user.version mismatch", lambda d: d["daemons"]["user"].update(version="0.0.0")),
            ("installed: daemons.user.protocol_version mismatch", lambda d: d["daemons"]["user"].update(protocol_version="2")),
        ]
        for name, mutate in cases:
            with self.subTest(invalid=name):
                self.assert_provenance_error(mutate)

    def test_feature_order_and_duplicates_rejected(self):
        for name, features in [
            ("unsorted", ["safe", "fast"]),
            ("duplicate", ["fast", "fast"]),
            ("empty", ["fast", ""]),
        ]:
            with self.subTest(features=name):
                data = make_input()
                data["build"]["features"] = features
                with self.assertRaises(ContractError):
                    validate_provenance(data)

    def test_endpoint_rejects_credential_markers_and_url_delimiters(self):
        for marker in ("://", "@", "?", "#", "key", "token", "secret", "password"):
            with self.subTest(marker=marker):
                data = make_input()
                data["provider"]["endpoint_id"] = "prefix" + marker + "suffix"
                with self.assertRaises(ContractError):
                    validate_provenance(data)

    def test_waiver_valid_unknown_missing_malformed_p0(self):
        result = validate_waiver(
            copy.deepcopy(VALID_WAIVER),
            reference_time=REFERENCE_TIME,
        )
        self.assertEqual(result, VALID_WAIVER)

        with self.assertRaises(ContractError):
            validate_waiver(
                copy.deepcopy(VALID_WAIVER),
                reference_time=datetime(2026, 1, 2, tzinfo=timezone.utc),
            )

        unknown = copy.deepcopy(VALID_WAIVER)
        unknown["extra"] = True
        with self.assertRaises(ContractError):
            validate_waiver(unknown)

        for key in VALID_WAIVER:
            with self.subTest(missing=key):
                missing = copy.deepcopy(VALID_WAIVER)
                del missing[key]
                with self.assertRaises(ContractError):
                    validate_waiver(missing)

        for expires_at in ("2026-01-01", "2026/01/01T00:00:00Z", "2026-01-01T00:00:00"):
            with self.subTest(expires_at=expires_at):
                malformed = copy.deepcopy(VALID_WAIVER)
                malformed["expires_at"] = expires_at
                with self.assertRaises(ContractError):
                    validate_waiver(malformed)

        with self.assertRaises(ContractError):
            validate_waiver(copy.deepcopy(VALID_WAIVER), p0=True)

    def test_canonical_bytes_deterministic_and_newline(self):
        obj = {"b": [2, 1], "a": "x"}
        self.assertEqual(canonical_bytes(obj), canonical_bytes({"a": "x", "b": [2, 1]}))
        self.assertTrue(canonical_bytes(obj).endswith(b"\n"))
        self.assertEqual(canonical_bytes(obj), b'{"a":"x","b":[2,1]}\n')

    def test_sha256_json_matches_hashlib(self):
        obj = {"z": 1, "a": ["x"], "n": None}
        self.assertEqual(sha256_json(obj), hashlib.sha256(canonical_bytes(obj)).hexdigest())

    def test_non_installed_environment_allows_different_daemon_identity(self):
        data = make_input()
        data["environment_level"] = "simulation"
        data["installed_artifact"]["path"] = "/other/bin"
        data["installed_artifact"]["sha256"] = "c" * 64
        data["daemons"]["machine"].update(
            path="/other/machine", sha256="d" * 64,
            version="2.0.0", protocol_version="2")
        data["daemons"]["user"].update(
            path="/other/user", sha256="f" * 64,
            version="3.0.0", protocol_version="3")
        result = validate_provenance(data)
        self.assertEqual(result["daemons"]["machine"]["path"], "/other/machine")
        self.assertEqual(result["daemons"]["user"]["protocol_version"], "3")
