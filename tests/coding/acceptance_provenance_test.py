"""Tests for acceptance provenance collection module."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import stat
import tempfile
import unittest
from collections import deque
from typing import Any, Dict, List, Optional, Tuple

from harness.acceptance_provenance import (
    ProvenanceCollectionError,
    CommandResult,
    collect_installed_provenance,
    INSTALLED_PATH,
    PROC_ROOT,
    _BUILD_FEATURES,
    _BUILD_PROFILE,
    _Collector,
    _EXEC_PATH_LITERAL,
    _InstallChecker,
    _MAX_CFG_OUT,
    _MAX_SYSTEMCTL_OUT,
    _MAX_VER_OUT,
    _PathMapper,
    _identity,
)

_FAKE_DIGEST = "f" * 64
_FAKE_GEN_ID = "test-gen-1"
_FAKE_SHA256 = "e" * 64
_FAKE_GIT_SHA = "a" * 40
_FAKE_VERSION = "3.2.1"
_FAKE_PROTO = 7
_HELLO_BYTES = b"hello aletheon\n"


def _fv(version=_FAKE_VERSION, proto=_FAKE_PROTO):
    return json.dumps({"schema_version": 1, "name": "aletheon", "version": version,
                       "protocol_version": proto}).encode()


def _fc(provider="test-provider", model="test-model", base_url="https://api.example.com/v1/"):
    return json.dumps({"agent": {"default_provider": provider, "default_model": model},
                       "providers": [{"name": provider, "base_url": base_url}]}).encode()


def _sc(main_pid=1234, nrestarts=0,
        execstart="{ path=/usr/bin/aletheon ; argv[]=/usr/bin/aletheon ; ignore_errors=no }",
        fragment="/etc/systemd/system/aletheon-core.service"):
    return (f"MainPID={main_pid}\nNRestarts={nrestarts}\n"
            f"ExecStart={execstart}\nFragmentPath={fragment}\n").encode()


class FakeRunner:
    """FIFO command runner with exact argv matching."""

    def __init__(self):
        self._queue: deque = deque()
        self.calls: List[Tuple[List[str], float, Optional[Dict]]] = []

    def enqueue(self, argv: List[str], result: CommandResult):
        self._queue.append((list(argv), result))

    def run(self, argv: List[str], *, timeout: float,
            env: Optional[Dict[str, str]] = None) -> CommandResult:
        self.calls.append((list(argv), timeout, dict(env) if env else None))
        if not self._queue:
            raise AssertionError(f"Unexpected run() argv={argv}; queue empty")
        expected_argv, result = self._queue.popleft()
        assert argv == expected_argv, f"argv mismatch: {expected_argv!r} vs {argv!r}"
        return result

    def assert_consumed(self):
        assert not self._queue, f"unconsumed: {list(self._queue)}"


def _ok(stdout=b"", stderr=b""):
    return CommandResult(returncode=0, stdout=stdout, stderr=stderr)


def _err(stdout=b"", stderr=b"", returncode=1):
    return CommandResult(returncode=returncode, stdout=stdout, stderr=stderr)


# ---------------------------------------------------------------------------
class _Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = pathlib.Path(self._tmp.name)
        self.ib = self.tmp / "backing"
        self.ib.write_bytes(_HELLO_BYTES)
        os.chmod(self.ib, 0o755)
        self.pr = self.tmp / "proc"
        self.pr.mkdir()
        self.pd = self.pr / "1234"
        self.pd.mkdir()
        (self.pd / "exe").symlink_to(self.ib)
        self.ff = self.tmp / "fragment.service"
        self.ff.write_text("[Service]\nExecStart=/usr/bin/aletheon\n")
        self.mapper = _PathMapper()
        self.mapper.installed_to_backing = lambda p: os.fspath(self.ib) if p == _EXEC_PATH_LITERAL else p
        self.mapper.fragment_to_backing = lambda p: os.fspath(self.ff) if os.path.isabs(p) and p.startswith("/etc/systemd/") else p
        self.mapper.proc_exe_to_logical = lambda p: _EXEC_PATH_LITERAL if os.fspath(self.ib) in p else p
        self.mapper.command_for = _identity
        self.checker = _InstallChecker()
        self.runner = FakeRunner()
        self.c = _Collector(self.runner, INSTALLED_PATH, self.pr, self.checker, self.mapper)

    def tearDown(self):
        self._tmp.cleanup()

    def _rd(self, name="repo"):
        d = self.tmp / name
        d.mkdir(exist_ok=True)
        (d / ".git").mkdir(exist_ok=True)
        return d

    def _qgit(self, root, sha=_FAKE_GIT_SHA, dirty=False):
        s = os.fspath(root)
        self.runner.enqueue(["git", "-C", s, "rev-parse", "--show-toplevel"], _ok(s.encode() + b"\n"))
        self.runner.enqueue(["git", "-C", s, "rev-parse", "HEAD"], _ok(sha.encode() + b"\n"))
        self.runner.enqueue(["git", "-C", s, "status", "--porcelain=v1", "--untracked-files=normal"],
                            _ok(b" M x.rs\n" if dirty else b""))

    def _qvc(self, version=_FAKE_VERSION, proto=_FAKE_PROTO, provider="test-provider",
             model="test-model", base_url="https://api.example.com/v1/"):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(_fv(version, proto)))
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(_fc(provider, model, base_url)))

    def _qsys(self, n=2):
        for _ in range(n):
            for unit, user in [("aletheon-core.service", False), ("aletheon.service", True),
                               ("aletheon-memory-agent.service", True)]:
                frag = f"/etc/systemd/{'user/' if user else 'system/'}{'aletheon' if 'core' not in unit else 'aletheon-core'}.service"
                argv = ["systemctl"] + (["--user"] if user else []) + [
                    "show", unit, "--property=MainPID", "--property=NRestarts",
                    "--property=ExecStart", "--property=FragmentPath"]
                self.runner.enqueue(argv, _ok(_sc(fragment=frag)))


# ===========================================================================
class TestFakeRunner(unittest.TestCase):
    def test_fifo_and_exact_argv(self):
        r = FakeRunner()
        r.enqueue(["a"], _ok(b"1"))
        r.enqueue(["b"], _err(b"2"))
        self.assertEqual(r.run(["a"], timeout=1).stdout, b"1")
        self.assertEqual(r.run(["b"], timeout=1).returncode, 1)
        r.assert_consumed()
        self.assertEqual(len(r.calls), 2)

    def test_unexpected_argv_raises(self):
        r = FakeRunner()
        r.enqueue(["x"], _ok())
        with self.assertRaises(AssertionError):
            r.run(["y"], timeout=1)

    def test_extra_call_raises(self):
        r = FakeRunner()
        r.enqueue(["x"], _ok())
        r.run(["x"], timeout=1)
        with self.assertRaises(AssertionError):
            r.run(["z"], timeout=1)


# ===========================================================================
class TestParams(_Base):
    def test_fixture_digest_rejected(self):
        for bad in ("abc", "g" * 64, "G" * 64, "f" * 63, "f" * 65, "", 12345):
            with self.subTest(digest=bad):
                r = FakeRunner()
                cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc.collect(bad, _FAKE_GEN_ID, self._rd())
                self.assertIn("fixture_digest", str(ctx.exception).lower())
                self.assertEqual(len(r.calls), 0)

    def test_generation_id_rejected(self):
        for bad in ("", "bad gen", "has space", "x" * 300, "with\nnewline"):
            with self.subTest(gen_id=bad):
                r = FakeRunner()
                cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc.collect(_FAKE_DIGEST, bad, self._rd())
                self.assertIn("generation_id", str(ctx.exception).lower())
                self.assertEqual(len(r.calls), 0)


# ===========================================================================
class TestRepoInfo(_Base):
    def test_repo_root_errors(self):
        # nonexistent
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(self.tmp / "noexist")
        self.assertIn("not a directory", str(ctx.exception).lower())

    def test_repo_root_regular_file(self):
        f = self.tmp / "regfile"
        f.write_text("not a directory")
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(f)
        self.assertIn("not a directory", str(ctx.exception).lower())
        self.assertEqual(len(self.runner.calls), 0)

    def test_git_toplevel_nonzero(self):
        d = self._rd()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _err())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("resolve git root", str(ctx.exception).lower())

    def test_git_toplevel_multiline(self):
        d = self._rd()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _ok(b"/a\n/b\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("malformed", str(ctx.exception).lower())

    def test_git_toplevel_mismatch(self):
        d = self._rd()
        o = self.tmp / "other"
        o.mkdir()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _ok(os.fspath(o).encode() + b"\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("does not match", str(ctx.exception).lower())

    def test_git_head_fails(self):
        d = self._rd()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _ok(os.fspath(d).encode() + b"\n"))
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "HEAD"], _err())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("git head", str(ctx.exception).lower())

    def test_git_head_bad_sha(self):
        d = self._rd()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _ok(os.fspath(d).encode() + b"\n"))
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "HEAD"], _ok(b"xyz\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("sha", str(ctx.exception).lower())

    def test_git_status_fails(self):
        d = self._rd()
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "--show-toplevel"], _ok(os.fspath(d).encode() + b"\n"))
        self.runner.enqueue(["git", "-C", os.fspath(d), "rev-parse", "HEAD"], _ok(_FAKE_GIT_SHA.encode() + b"\n"))
        self.runner.enqueue(["git", "-C", os.fspath(d), "status", "--porcelain=v1", "--untracked-files=normal"], _err())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(d)
        self.assertIn("status", str(ctx.exception).lower())

    def test_repo_info_clean_and_dirty(self):
        for idx, (dirty, exp) in enumerate([(False, False), (True, True)]):
            with self.subTest(dirty=dirty):
                d = self._rd(f"r{idx}")
                self._qgit(d, dirty=dirty)
                info = self.c._repo_info(d)
                self.assertEqual(info["sha"], _FAKE_GIT_SHA)
                self.assertEqual(info["dirty"], exp)


# ===========================================================================
class TestInstallArtifact(_Base):
    def test_symlink_rejected(self):
        ln = self.tmp / "link"
        ln.symlink_to(self.ib)
        m = _PathMapper()
        m.installed_to_backing = lambda p: os.fspath(ln) if p == _EXEC_PATH_LITERAL else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, _InstallChecker(), m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._install_artifact()
        self.assertIn("regular file", str(ctx.exception).lower())

    def test_noexec_rejected(self):
        ne = self.tmp / "noexec"
        ne.write_bytes(_HELLO_BYTES)
        os.chmod(ne, 0o644)
        m = _PathMapper()
        m.installed_to_backing = lambda p: os.fspath(ne) if p == _EXEC_PATH_LITERAL else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, _InstallChecker(), m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._install_artifact()
        self.assertIn("executable", str(ctx.exception).lower())

    def test_race_during_collection(self):
        orig = self.tmp / "orig"
        orig.write_bytes(_HELLO_BYTES)
        os.chmod(orig, 0o755)
        swp = self.tmp / "swp"
        swp.write_bytes(b"diff")
        chk = _InstallChecker()
        chk._pre_open = lambda p: os.replace(swp, p)
        m = _PathMapper()
        m.installed_to_backing = lambda p: os.fspath(orig) if p == _EXEC_PATH_LITERAL else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, chk, m)
        with self.assertRaises(ProvenanceCollectionError):
            cc._install_artifact()

    def test_backing_directory_rejected(self):
        d = self.tmp / "mydir"
        d.mkdir()
        m = _PathMapper()
        m.installed_to_backing = lambda p: os.fspath(d) if p == _EXEC_PATH_LITERAL else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, _InstallChecker(), m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._install_artifact()
        self.assertIn("regular file", str(ctx.exception).lower())

    def test_digest_correct(self):
        self.assertEqual(self.c._install_artifact(), hashlib.sha256(_HELLO_BYTES).hexdigest())


# ===========================================================================
class TestVersionInfo(_Base):
    def test_nonzero_return(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _err())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("version", str(ctx.exception).lower())

    def test_not_json(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(b"not json"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("json", str(ctx.exception).lower())

    def test_wrong_keys(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"],
                            _ok(b'{"schema_version":1,"name":"aletheon"}'))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("schema mismatch", str(ctx.exception).lower())

    def test_schema_version_type(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"],
                            _ok(b'{"schema_version":"1","name":"aletheon","version":"1.0","protocol_version":1}'))
        with self.assertRaises(ProvenanceCollectionError):
            self.c._version_info()

    def test_name_not_aletheon(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"],
                            _ok(b'{"schema_version":1,"name":"other","version":"1.0","protocol_version":1}'))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("name mismatch", str(ctx.exception).lower())

    def test_version_validation(self):
        cases = [
            ("v" * 300, "too long"),
            ("", "version"), ("   ", "whitespace"), ("v\x01", "control"),
        ]
        for ver, substr in cases:
            with self.subTest(ver=ver):
                r = FakeRunner()
                r.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(_fv(version=ver)))
                cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc._version_info()
                self.assertIn(substr, str(ctx.exception).lower())

    def test_protocol_version_invalid(self):
        for pv in (0, -1):
            with self.subTest(proto=pv):
                r = FakeRunner()
                r.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(_fv(proto=pv)))
                cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc._version_info()
                self.assertIn("protocol", str(ctx.exception).lower())

    def test_happy(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(_fv("2.0.0", 5)))
        v, p = self.c._version_info()
        self.assertEqual((v, p), ("2.0.0", "5"))


# ===========================================================================
class TestConfigProvider(_Base):
    def test_subcommand_fails(self):
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _err())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._config_provider()
        self.assertIn("config", str(ctx.exception).lower())

    def test_not_json(self):
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(b"nope"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._config_provider()
        self.assertIn("json", str(ctx.exception).lower())

    def test_missing_agent(self):
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(b'{"providers":[]}'))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._config_provider()
        self.assertIn("agent config", str(ctx.exception).lower())

    def test_provider_not_found(self):
        cfg = json.dumps({"agent": {"default_provider": "x", "default_model": "m"},
                          "providers": [{"name": "y", "base_url": "https://a.com/"}]}).encode()
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(cfg))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._config_provider()
        self.assertIn("ambiguous", str(ctx.exception).lower())

    def test_no_base_url(self):
        cfg = json.dumps({"agent": {"default_provider": "p", "default_model": "m"},
                          "providers": [{"name": "p"}]}).encode()
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(cfg))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._config_provider()
        self.assertIn("base_url", str(ctx.exception).lower())

    def test_happy_endpoint_hashed(self):
        self.runner.enqueue(["/usr/bin/aletheon", "config", "effective"],
                            _ok(_fc("prov", "model", "https://api.example.com/v1/")))
        r = self.c._config_provider()
        self.assertEqual(r["provider_id"], "prov")
        self.assertEqual(r["model_id"], "model")
        self.assertEqual(r["endpoint_id"], "sha256-" + hashlib.sha256(b"https://api.example.com/v1/").hexdigest())


# ===========================================================================
class TestEndpoint(_Base):
    def test_rejections(self):
        cases = [
            ("ftp://x.com/", "scheme"),
            ("https:///p/", "hostname"),
            ("https://u:p@x.com/", "credentials"),
            ("https://x.com/?q=1", "query"),
            ("https://x.com/#f", "fragment"),
            ("https://x.com/../a", "dot segment"),
            ("https://x.com/\x01", "control"),
            ("https://-x.com/", "hostname label"),
        ]
        for url, sub in cases:
            with self.subTest(url=url):
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    self.c._normalise_endpoint(url)
                self.assertIn(sub, str(ctx.exception).lower())

    def test_redacted_rejected(self):
        for t in ("redacted", "<redacted>", "key", "token", "secret", "password"):
            with self.subTest(token=t):
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    self.c._normalise_endpoint(f"https://x.com/{t}/")
                self.assertIn("placeholder", str(ctx.exception).lower())

    def test_invalid_port_non_numeric(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._normalise_endpoint("https://example.com:bad")
        self.assertIn("port", str(ctx.exception).lower())

    def test_malformed_percent_ZZ_passes(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._normalise_endpoint("https://example.com/%ZZ/")
        self.assertIn("encoding", str(ctx.exception).lower())

    def test_hostname_label_too_long(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._normalise_endpoint(f"https://{'a'*64}.com/")
        self.assertIn("international hostname", str(ctx.exception).lower())

    def test_base_url_too_long(self):
        with self.assertRaises(ProvenanceCollectionError):
            self.c._normalise_endpoint("https://example.com/" + "a" * 4096)

    def test_normalisation(self):
        self.assertEqual(self.c._normalise_endpoint("https://example.com"), "https://example.com/")
        self.assertEqual(self.c._normalise_endpoint("https://example.com:443/"), "https://example.com/")
        self.assertEqual(self.c._normalise_endpoint("http://example.com:80/path/"), "http://example.com/path/")


# ===========================================================================
class TestExecStart(_Base):
    def test_literal(self):
        self.assertEqual(self.c._parse_execstart("/usr/bin/aletheon --flag"), _EXEC_PATH_LITERAL)

    def test_structured(self):
        self.assertEqual(self.c._parse_execstart(
            "{ path=/usr/bin/aletheon ; argv[]=/usr/bin/aletheon ; ignore_errors=no }"), _EXEC_PATH_LITERAL)

    def test_rejections(self):
        cases = [
            ("{ path=/usr/bin/aletheon ; path=/other ; }", "exactly one path"),
            ("{ ignore_errors=no }", "exactly one path"),
            ("@something", "unrecognised"),
            ("{ path=/usr/bin/aletheon ;", "malformed"),
            ("/usr/bin/aletheon\x01", "control"),
            ("a" * 5000, "too long"),
        ]
        for val, sub in cases:
            with self.subTest(val=val[:40]):
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    self.c._parse_execstart(val)
                self.assertIn(sub, str(ctx.exception).lower())

    def test_wrong_path(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._parse_execstart("/usr/bin/other --flag")
        self.assertIn("unrecognised", str(ctx.exception).lower())
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._parse_execstart("{ path=/usr/bin/other ; ignore_errors=no }")
        self.assertIn("not /usr/bin/aletheon", str(ctx.exception).lower())


# ===========================================================================
class TestFragment(_Base):
    def test_not_absolute(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._check_fragment("rel/path")
        self.assertIn("not absolute", str(ctx.exception).lower())

    def test_symlink_rejected(self):
        ln = self.tmp / "flink"
        ln.symlink_to(self.ff)
        m = _PathMapper()
        m.fragment_to_backing = lambda p: os.fspath(ln) if p.startswith("/etc/") else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, self.checker, m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._check_fragment("/etc/systemd/system/t.service")
        self.assertIn("symbolic link", str(ctx.exception).lower())

    def test_non_regular(self):
        d = self.tmp / "fdir"
        d.mkdir()
        m = _PathMapper()
        m.fragment_to_backing = lambda p: os.fspath(d) if p.startswith("/etc/") else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, self.checker, m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._check_fragment("/etc/t.service")
        self.assertIn("not a regular file", str(ctx.exception).lower())

    def test_missing(self):
        m = _PathMapper()
        m.fragment_to_backing = lambda p: os.fspath(self.tmp / "no") if p.startswith("/etc/") else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, self.checker, m)
        with self.assertRaises(ProvenanceCollectionError):
            cc._check_fragment("/etc/m.service")

    def test_valid(self):
        self.c._check_fragment("/etc/systemd/system/aletheon-core.service")


# ===========================================================================
class TestProcExe(_Base):
    def test_fallback_on_missing(self):
        (self.pd / "exe").unlink()
        self.assertEqual(self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL), _EXEC_PATH_LITERAL)

    def test_relative_rejected(self):
        (self.pd / "exe").unlink()
        (self.pd / "exe").symlink_to("rel/path")
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL)
        self.assertIn("relative", str(ctx.exception).lower())

    def test_deleted_rejected(self):
        (self.pd / "exe").unlink()
        (self.pd / "exe").symlink_to("/usr/bin/aletheon (deleted)")
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL)
        self.assertIn("deleted", str(ctx.exception).lower())

    def test_not_aletheon_rejected(self):
        o = self.tmp / "other"
        o.write_bytes(b"x")
        os.chmod(o, 0o755)
        (self.pd / "exe").unlink()
        (self.pd / "exe").symlink_to(o)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL)
        self.assertIn("not /usr/bin/aletheon", str(ctx.exception).lower())

    def test_control_chars_rejected(self):
        (self.pd / "exe").unlink()
        (self.pd / "exe").symlink_to("/usr/bin/aletheon\x01")
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL)
        self.assertIn("control", str(ctx.exception).lower())

    def test_identity(self):
        self.assertEqual(self.c._resolve_daemon_exe("1234", _EXEC_PATH_LITERAL), _EXEC_PATH_LITERAL)


# ===========================================================================
class TestSystemd(_Base):
    def _qs(self, unit, user, result):
        argv = ["systemctl"] + (["--user"] if user else []) + [
            "show", unit, "--property=MainPID", "--property=NRestarts",
            "--property=ExecStart", "--property=FragmentPath"]
        self.runner.enqueue(argv, result)

    def test_nonzero(self):
        self._qs("test.service", False, _err(b"fail"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("test.service")
        self.assertIn("failed", str(ctx.exception).lower())

    def test_empty(self):
        self.runner.enqueue(["systemctl", "show", "t.service", "--property=MainPID",
                              "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"], _ok(b""))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("t.service")
        self.assertIn("empty", str(ctx.exception).lower())

    def test_missing_prop(self):
        self.runner.enqueue(["systemctl", "show", "t.service", "--property=MainPID",
                              "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"],
                            _ok(b"MainPID=1\nNRestarts=0\nExecStart=/usr/bin/aletheon\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("t.service")
        self.assertIn("missing", str(ctx.exception).lower())

    def test_duplicate(self):
        self.runner.enqueue(["systemctl", "show", "t.service", "--property=MainPID",
                              "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"],
                            _ok(b"MainPID=1\nMainPID=2\nNRestarts=0\nExecStart=/usr/bin/aletheon\nFragmentPath=/e/x\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("t.service")
        self.assertIn("duplicate", str(ctx.exception).lower())

    def test_unexpected_prop(self):
        self.runner.enqueue(["systemctl", "show", "t.service", "--property=MainPID",
                              "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"],
                            _ok(b"MainPID=1\nNRestarts=0\nExecStart=/usr/bin/aletheon\nFragmentPath=/e/x\nBad=1\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("t.service")
        self.assertIn("unexpected", str(ctx.exception).lower())

    def test_control_in_output(self):
        self.runner.enqueue(["systemctl", "show", "t.service", "--property=MainPID",
                              "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"],
                            _ok(b"MainPID=1\nNRestarts=0\nExecStart=/usr/bin/aletheon\x01\nFragmentPath=/e/x\n"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._systemctl_show("t.service")
        self.assertIn("control", str(ctx.exception).lower())

    def test_invalid_pid_and_nrestarts(self):
        for label, pid, nr, sub in [("neg pid", -1, 0, "mainpid"), ("zero pid", 0, 0, "mainpid"),
                                      ("neg nr", 1, -5, "nrestarts")]:
            with self.subTest(case=label):
                r = FakeRunner()
                cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
                argv = ["systemctl", "show", "u.service", "--property=MainPID",
                        "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
                r.enqueue(argv, _ok(_sc(main_pid=pid, nrestarts=nr)))
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc._daemon_snapshot("u.service", False)
                self.assertIn(sub, str(ctx.exception).lower())

    def test_happy_snapshot(self):
        self._qs("aletheon-core.service", False, _ok(_sc(fragment="/etc/systemd/system/aletheon-core.service")))
        snap = self.c._daemon_snapshot("aletheon-core.service", False)
        self.assertEqual(snap, {"MainPID": 1234, "NRestarts": 0})


# ===========================================================================
class TestCmdValidation(_Base):
    def test_stdout_too_large(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(b"x" * (_MAX_VER_OUT + 1)))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("exceeded limit", str(ctx.exception).lower())

    def test_stderr_too_large(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"],
                            CommandResult(0, b"{}", b"x" * 65537))
        with self.assertRaises(ProvenanceCollectionError):
            self.c._version_info()

    def test_non_bytes_stdout(self):
        r = FakeRunner()
        r.enqueue(["/usr/bin/aletheon", "version", "--json"], CommandResult(0, "str", b""))  # type: ignore
        cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._version_info()
        self.assertIn("stdout type", str(ctx.exception).lower())

    def test_non_int_returncode(self):
        r = FakeRunner()
        r.enqueue(["/usr/bin/aletheon", "version", "--json"], CommandResult("0", b"{}", b""))  # type: ignore
        cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._version_info()
        self.assertIn("return code", str(ctx.exception).lower())

    def test_bool_returncode(self):
        r = FakeRunner()
        r.enqueue(["/usr/bin/aletheon", "version", "--json"], CommandResult(False, b"{}", b""))  # type: ignore
        cc = _Collector(r, INSTALLED_PATH, self.pr, self.checker, self.mapper)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._version_info()
        self.assertIn("return code", str(ctx.exception).lower())

    def test_non_utf8(self):
        self.runner.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(b"\xff\xfe"))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._version_info()
        self.assertIn("utf-8", str(ctx.exception).lower())


# ===========================================================================
class TestBuildInfo(_Base):
    def test_build_info(self):
        info = self.c._build_info()
        self.assertEqual(info["profile"], _BUILD_PROFILE)
        self.assertEqual(info["features"], sorted(_BUILD_FEATURES))


# ===========================================================================
class TestSnapshotStability(_Base):
    def _mk_collector(self, repo_root, systemctl_results):
        r = FakeRunner()
        m = _PathMapper()
        m.fragment_to_backing = lambda p: os.fspath(self.ff) if os.path.isabs(p) and p.startswith("/etc/systemd/") else p
        m.installed_to_backing = lambda p: os.fspath(self.ib) if p == _EXEC_PATH_LITERAL else p
        m.proc_exe_to_logical = lambda p: _EXEC_PATH_LITERAL if os.fspath(self.ib) in p else p
        m.command_for = _identity
        s = os.fspath(repo_root)
        r.enqueue(["git", "-C", s, "rev-parse", "--show-toplevel"], _ok(s.encode() + b"\n"))
        r.enqueue(["git", "-C", s, "rev-parse", "HEAD"], _ok(_FAKE_GIT_SHA.encode() + b"\n"))
        r.enqueue(["git", "-C", s, "status", "--porcelain=v1", "--untracked-files=normal"], _ok(b""))
        r.enqueue(["/usr/bin/aletheon", "version", "--json"], _ok(_fv()))
        r.enqueue(["/usr/bin/aletheon", "config", "effective"], _ok(_fc()))
        for argv, res in systemctl_results:
            r.enqueue(argv, res)
        return _Collector(r, INSTALLED_PATH, self.pr, self.checker, m)

    def test_machine_pid_change_detected(self):
        repo = self._rd("rp")
        am = ["systemctl", "show", "aletheon-core.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        au = ["systemctl", "--user", "show", "aletheon.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        ame = ["systemctl", "--user", "show", "aletheon-memory-agent.service", "--property=MainPID",
                "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        results = [
            (am, _ok(_sc(main_pid=100))), (au, _ok(_sc(main_pid=200))), (ame, _ok(_sc(main_pid=300))),
            (am, _ok(_sc(main_pid=999))), (au, _ok(_sc(main_pid=200))), (ame, _ok(_sc(main_pid=300))),
        ]
        cc = self._mk_collector(repo, results)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc.collect(_FAKE_DIGEST, _FAKE_GEN_ID, repo)
        self.assertIn("machine", str(ctx.exception).lower())

    def test_memory_nrestarts_change_detected(self):
        repo = self._rd("rn")
        am = ["systemctl", "show", "aletheon-core.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        au = ["systemctl", "--user", "show", "aletheon.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        ame = ["systemctl", "--user", "show", "aletheon-memory-agent.service", "--property=MainPID",
                "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        results = [
            (am, _ok(_sc())), (au, _ok(_sc())), (ame, _ok(_sc(nrestarts=0))),
            (am, _ok(_sc())), (au, _ok(_sc())), (ame, _ok(_sc(nrestarts=5))),
        ]
        cc = self._mk_collector(repo, results)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc.collect(_FAKE_DIGEST, _FAKE_GEN_ID, repo)
        self.assertIn("memory", str(ctx.exception).lower())

    def test_user_pid_change_detected(self):
        repo = self._rd("ru")
        am = ["systemctl", "show", "aletheon-core.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        au = ["systemctl", "--user", "show", "aletheon.service", "--property=MainPID",
               "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        ame = ["systemctl", "--user", "show", "aletheon-memory-agent.service", "--property=MainPID",
                "--property=NRestarts", "--property=ExecStart", "--property=FragmentPath"]
        results = [
            (am, _ok(_sc())), (au, _ok(_sc(main_pid=200))), (ame, _ok(_sc())),
            (am, _ok(_sc())), (au, _ok(_sc(main_pid=999))), (ame, _ok(_sc())),
        ]
        cc = self._mk_collector(repo, results)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc.collect(_FAKE_DIGEST, _FAKE_GEN_ID, repo)
        self.assertIn("user", str(ctx.exception).lower())


# ===========================================================================
class TestHappyFull(_Base):
    def test_full_collection(self):
        repo = self._rd()
        self._qgit(repo)
        self._qvc()
        self._qsys(n=2)
        result = self.c.collect(_FAKE_DIGEST, _FAKE_GEN_ID, repo)
        self.runner.assert_consumed()
        self.assertEqual(set(result.keys()), {
            "repo", "build", "environment_level", "installed_artifact",
            "client", "daemons", "provider", "fixture_digest", "generation_id"})
        self.assertEqual(result["repo"], {"sha": _FAKE_GIT_SHA, "dirty": False})
        self.assertEqual(result["build"]["profile"], _BUILD_PROFILE)
        self.assertEqual(result["build"]["features"], sorted(_BUILD_FEATURES))
        self.assertEqual(result["environment_level"], "installed")
        self.assertEqual(result["installed_artifact"]["path"], _EXEC_PATH_LITERAL)
        self.assertEqual(result["installed_artifact"]["sha256"], hashlib.sha256(_HELLO_BYTES).hexdigest())
        self.assertEqual(result["client"]["version"], _FAKE_VERSION)
        self.assertEqual(result["client"]["protocol_version"], str(_FAKE_PROTO))
        for key in ("machine", "user"):
            d = result["daemons"][key]
            self.assertEqual(d["path"], _EXEC_PATH_LITERAL)
            self.assertEqual(d["sha256"], hashlib.sha256(_HELLO_BYTES).hexdigest())
            self.assertEqual(d["version"], _FAKE_VERSION)
            self.assertEqual(d["protocol_version"], str(_FAKE_PROTO))
        self.assertEqual(result["provider"]["provider_id"], "test-provider")
        self.assertEqual(result["provider"]["model_id"], "test-model")
        self.assertEqual(result["provider"]["endpoint_id"],
                         "sha256-" + hashlib.sha256(b"https://api.example.com/v1/").hexdigest())
        self.assertEqual(result["fixture_digest"], _FAKE_DIGEST)
        self.assertEqual(result["generation_id"], _FAKE_GEN_ID)


# ===========================================================================
class TestPublicGuards(_Base):
    def test_wrong_installed_path(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            collect_installed_provenance(self._rd(), _FAKE_DIGEST, _FAKE_GEN_ID,
                                         installed_path=pathlib.Path("/tmp/x"))
        self.assertIn("/usr/bin/aletheon", str(ctx.exception).lower())

    def test_wrong_proc_root(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            collect_installed_provenance(self._rd(), _FAKE_DIGEST, _FAKE_GEN_ID,
                                         proc_root=pathlib.Path("/tmp/x"))
        self.assertIn("/proc", str(ctx.exception).lower())

    def test_defaults(self):
        self.assertEqual(INSTALLED_PATH, pathlib.Path("/usr/bin/aletheon"))
        self.assertEqual(PROC_ROOT, pathlib.Path("/proc"))


# ===========================================================================
class TestSanitisation(_Base):
    def test_error_no_temp_path(self):
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._repo_info(self.tmp / "noexist")
        self.assertNotIn(str(self.tmp), str(ctx.exception))

    def test_error_no_backing_path(self):
        ne = self.tmp / "noexec2"
        ne.write_bytes(_HELLO_BYTES)
        os.chmod(ne, 0o644)
        m = _PathMapper()
        m.installed_to_backing = lambda p: os.fspath(ne) if p == _EXEC_PATH_LITERAL else p
        cc = _Collector(self.runner, INSTALLED_PATH, self.pr, _InstallChecker(), m)
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            cc._install_artifact()
        self.assertNotIn(str(ne), str(ctx.exception))


# ===========================================================================
class TestCmdExceptions(unittest.TestCase):
    def test_wrapping(self):
        cases = [
            (TimeoutError("t"), "timed out"),
            (FileNotFoundError("f"), "not found"),
            (PermissionError("p"), "permission denied"),
            (OSError("o"), "os error"),
            (RuntimeError("r"), "execution failed"),
        ]
        for exc, sub in cases:
            with self.subTest(exc=type(exc).__name__):
                class R:
                    def run(self, argv, *, timeout, env=None):
                        raise exc
                cc = _Collector(R(), INSTALLED_PATH, pathlib.Path("/proc"), _InstallChecker(), _PathMapper())
                with self.assertRaises(ProvenanceCollectionError) as ctx:
                    cc._run_cmd(["cmd"])
                self.assertIn(sub, str(ctx.exception).lower())


# ===========================================================================
class TestRunCmdLimits(_Base):
    def test_max_stdout(self):
        self.runner.enqueue(["t"], _ok(b"x" * 10))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._run_cmd(["t"], max_stdout=5)
        self.assertIn("exceeded limit", str(ctx.exception).lower())

    def test_max_stderr(self):
        self.runner.enqueue(["t"], CommandResult(0, b"", b"y" * 100))
        with self.assertRaises(ProvenanceCollectionError) as ctx:
            self.c._run_cmd(["t"], max_stderr=10)
        self.assertIn("exceeded limit", str(ctx.exception).lower())


# ===========================================================================
class TestErrorType(unittest.TestCase):
    def test_is_value_error(self):
        self.assertTrue(issubclass(ProvenanceCollectionError, ValueError))
        try:
            raise ProvenanceCollectionError("x")
        except ValueError:
            pass
        else:
            self.fail("not caught as ValueError")


if __name__ == "__main__":
    unittest.main()
