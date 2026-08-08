from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import stat
import subprocess
import urllib.parse
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional, Protocol, Tuple

INSTALLED_PATH = pathlib.Path("/usr/bin/aletheon")
PROC_ROOT = pathlib.Path("/proc")

# Build profile and features as reviewed in scripts/lib/aletheon/build.sh:3-19
_BUILD_PROFILE = "release"
_BUILD_FEATURES: List[str] = []

_MAX_VER_OUT = 64 * 1024
_MAX_CFG_OUT = 2 * 1024 * 1024
_MAX_SYSTEMCTL_OUT = 1024 * 1024

_HEX_RE = re.compile(r"^[0-9a-f]+$")
_GENERATION_ID_RE = re.compile(r"^[a-zA-Z0-9._-]+$")
_EXEC_PATH_LITERAL = "/usr/bin/aletheon"

_PID_RE = re.compile(r"^[1-9][0-9]{0,9}$")
_RESTARTS_RE = re.compile(r"^(?:0|[1-9][0-9]{0,9})$")


class ProvenanceCollectionError(ValueError):
    """Raised when provenance collection fails with a sanitised message."""


@dataclass(frozen=True, slots=True)
class CommandResult:
    returncode: int
    stdout: bytes
    stderr: bytes


class CommandRunner(Protocol):
    def run(self, argv: List[str], *, timeout: float, env: Optional[Dict[str, str]] = None) -> CommandResult:
        ...


def _sanitise_exc(msg: str) -> ProvenanceCollectionError:
    return ProvenanceCollectionError(msg)


def _validate_hex(s: str, lengths: Tuple[int, ...] = (40, 64)) -> bool:
    if not isinstance(s, str):
        return False
    return len(s) in lengths and bool(_HEX_RE.match(s))


def _validate_generation_id(s: str) -> bool:
    if not isinstance(s, str):
        return False
    return bool(s) and len(s) <= 256 and bool(_GENERATION_ID_RE.match(s))


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _sha256_file(fd: int) -> str:
    os.lseek(fd, 0, os.SEEK_SET)
    h = hashlib.sha256()
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            break
        h.update(chunk)
    return h.hexdigest()


def _ensure_regular_executable(st_mode: int) -> None:
    if not stat.S_ISREG(st_mode):
        raise _sanitise_exc("Installed artifact is not a regular file")
    if not (st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)):
        raise _sanitise_exc("Installed artifact lacks executable permission bits")


def _identity(x: str) -> str:
    return x


def _default_hook(*args: Any, **kwargs: Any) -> None:
    pass


class _InstallChecker:
    """Verifies the installed artifact safely, with injection points for tests."""
    _pre_open: Callable[..., None] = _default_hook
    _post_open: Callable[..., None] = _default_hook
    _post_close: Callable[..., None] = _default_hook

    def verify(self, path: pathlib.Path) -> str:
        p = os.fspath(path)
        try:
            st_before = os.lstat(p)
            _ensure_regular_executable(st_before.st_mode)
            self._pre_open(p)
            fd = os.open(p, os.O_RDONLY | os.O_CLOEXEC | getattr(os, 'O_NOFOLLOW', 0))
            try:
                self._post_open(fd, p)
                fst_open = os.fstat(fd)
                digest = _sha256_file(fd)
                fst_after = os.fstat(fd)
            finally:
                os.close(fd)
                self._post_close(p)
            st_after = os.lstat(p)
        except OSError:
            raise _sanitise_exc("Installed artifact verification failed") from None

        # identity checks
        if not (st_before.st_dev == fst_open.st_dev == fst_after.st_dev == st_after.st_dev):
            raise _sanitise_exc("Installed artifact device mismatch")
        if not (st_before.st_ino == fst_open.st_ino == fst_after.st_ino == st_after.st_ino):
            raise _sanitise_exc("Installed artifact inode mismatch")
        if not (st_before.st_mode == fst_open.st_mode == fst_after.st_mode == st_after.st_mode):
            raise _sanitise_exc("Installed artifact mode changed during collection")
        if not (st_before.st_size == fst_open.st_size == fst_after.st_size == st_after.st_size):
            raise _sanitise_exc("Installed artifact size changed during collection")
        if not (st_before.st_mtime_ns == fst_open.st_mtime_ns == fst_after.st_mtime_ns == st_after.st_mtime_ns):
            raise _sanitise_exc("Installed artifact mtime changed during collection")
        return digest


class _PathMapper:
    """Production identity mapper; test seam may override mappings."""

    def __init__(self) -> None:
        self.installed_to_backing: Callable[[str], str] = _identity
        self.fragment_to_backing: Callable[[str], str] = _identity
        self.proc_exe_to_logical: Callable[[str], str] = _identity
        self.command_for: Callable[[str], str] = _identity


class _ProductionDependencies:
    install_checker: _InstallChecker = _InstallChecker()
    path_mapper: _PathMapper = _PathMapper()


class _Collector:
    def __init__(
        self,
        runner: CommandRunner,
        installed_path: pathlib.Path,
        proc_root: pathlib.Path,
        install_checker: _InstallChecker,
        path_mapper: _PathMapper,
    ) -> None:
        self._runner = runner
        self._installed = installed_path
        self._proc_root = proc_root
        self._install_checker = install_checker
        self._mapper = path_mapper

    def _run_cmd(self, argv: List[str], *, timeout: float = 30.0,
                 max_stdout: int = _MAX_CFG_OUT, max_stderr: int = 65536) -> CommandResult:
        try:
            res = self._runner.run(argv, timeout=timeout)
        except TimeoutError:
            raise _sanitise_exc("Command timed out") from None
        except FileNotFoundError:
            raise _sanitise_exc("Command not found") from None
        except PermissionError:
            raise _sanitise_exc("Permission denied for command") from None
        except OSError:
            raise _sanitise_exc("OS error running command") from None
        except Exception:
            raise _sanitise_exc("Command execution failed") from None

        if not isinstance(res.returncode, int) or isinstance(res.returncode, bool):
            raise _sanitise_exc("Invalid command return code")
        if not isinstance(res.stdout, bytes):
            raise _sanitise_exc("Invalid command stdout type")
        if not isinstance(res.stderr, bytes):
            raise _sanitise_exc("Invalid command stderr type")

        if len(res.stdout) > max_stdout:
            raise _sanitise_exc("Command output exceeded limit")
        if len(res.stderr) > max_stderr:
            raise _sanitise_exc("Command error output exceeded limit")
        try:
            res.stdout.decode("utf-8")
            res.stderr.decode("utf-8")
        except UnicodeDecodeError:
            raise _sanitise_exc("Command output is not valid UTF-8") from None
        return res

    def _resolve_git_root(self, repo_root: pathlib.Path) -> pathlib.Path:
        try:
            root = repo_root.resolve(strict=True)
        except OSError:
            raise _sanitise_exc("Repository root is not a directory") from None
        if not root.is_dir():
            raise _sanitise_exc("Repository root is not a directory")
        res = self._run_cmd(["git", "-C", os.fspath(root), "rev-parse", "--show-toplevel"],
                            timeout=15)
        if res.returncode != 0:
            raise _sanitise_exc("Failed to resolve git root")
        out = res.stdout.decode().strip()
        if not out or '\n' in out:
            raise _sanitise_exc("Git root output malformed")
        try:
            resolved_toplevel = pathlib.Path(out).resolve()
        except OSError:
            raise _sanitise_exc("Resolved git root path invalid") from None
        if root != resolved_toplevel:
            raise _sanitise_exc("Resolved git root does not match requested root")
        return root

    def _repo_info(self, repo_root: pathlib.Path) -> Dict[str, Any]:
        root = self._resolve_git_root(repo_root)
        # sha
        out = self._run_cmd(["git", "-C", os.fspath(root), "rev-parse", "HEAD"], timeout=15)
        if out.returncode != 0:
            raise _sanitise_exc("Failed to obtain git HEAD")
        sha = out.stdout.decode().strip()
        if not _validate_hex(sha, (40, 64)):
            raise _sanitise_exc("Invalid git HEAD SHA")
        # dirty
        stat_res = self._run_cmd(
            ["git", "-C", os.fspath(root), "status", "--porcelain=v1", "--untracked-files=normal"],
            timeout=15,
        )
        if stat_res.returncode != 0:
            raise _sanitise_exc("Failed to obtain git status")
        dirty = bool(stat_res.stdout.decode().strip())
        return {"sha": sha, "dirty": dirty}

    def _build_info(self) -> Dict[str, Any]:
        return {"profile": _BUILD_PROFILE, "features": sorted(_BUILD_FEATURES)}

    def _install_artifact(self) -> str:
        backing = pathlib.Path(self._mapper.installed_to_backing(os.fspath(self._installed)))
        return self._install_checker.verify(backing)

    def _version_info(self) -> Tuple[str, str]:
        executable = self._mapper.command_for(os.fspath(self._installed))
        argv = [executable, "version", "--json"]
        res = self._run_cmd(argv, timeout=30, max_stdout=_MAX_VER_OUT)
        if res.returncode != 0:
            raise _sanitise_exc("Version subcommand failed")
        try:
            data = json.loads(res.stdout.decode("utf-8"))
        except json.JSONDecodeError:
            raise _sanitise_exc("Version output is not valid JSON") from None
        if not isinstance(data, dict):
            raise _sanitise_exc("Version output is not a JSON object")
        if set(data.keys()) != {"schema_version", "name", "version", "protocol_version"}:
            raise _sanitise_exc("Version schema mismatch")
        schema_version = data["schema_version"]
        if isinstance(schema_version, bool) or not isinstance(schema_version, int) or schema_version != 1:
            raise _sanitise_exc("Invalid schema version")
        if data["name"] != "aletheon":
            raise _sanitise_exc("Version name mismatch")
        version = data["version"]
        if not isinstance(version, str) or not version:
            raise _sanitise_exc("Invalid version string")
        if len(version) > 256:
            raise _sanitise_exc("Version string too long")
        if not version.strip():
            raise _sanitise_exc("Version string is whitespace-only")
        if any(ord(c) < 32 or ord(c) == 127 for c in version):
            raise _sanitise_exc("Version contains control characters")
        proto = data["protocol_version"]
        if isinstance(proto, bool) or not isinstance(proto, int) or proto <= 0:
            raise _sanitise_exc("Invalid protocol version")
        return (version, str(proto))

    def _config_provider(self) -> Dict[str, str]:
        executable = self._mapper.command_for(os.fspath(self._installed))
        argv = [executable, "config", "effective"]
        res = self._run_cmd(argv, timeout=30, max_stdout=_MAX_CFG_OUT)
        if res.returncode != 0:
            raise _sanitise_exc("Effective config subcommand failed")
        try:
            cfg = json.loads(res.stdout.decode("utf-8"))
        except json.JSONDecodeError:
            raise _sanitise_exc("Config output is not valid JSON") from None
        if not isinstance(cfg, dict):
            raise _sanitise_exc("Config output is not a JSON object")
        agent = cfg.get("agent")
        if not isinstance(agent, dict):
            raise _sanitise_exc("Missing or invalid agent config")
        if "default_provider" not in agent or "default_model" not in agent:
            raise _sanitise_exc("Incomplete agent config")
        default_provider = agent["default_provider"]
        default_model = agent["default_model"]
        if not isinstance(default_provider, str) or not default_provider:
            raise _sanitise_exc("Invalid default provider")
        if len(default_provider) > 256:
            raise _sanitise_exc("Default provider name too long")
        if any(ord(c) < 32 or ord(c) == 127 for c in default_provider):
            raise _sanitise_exc("Default provider contains control characters")
        if not isinstance(default_model, str) or not default_model:
            raise _sanitise_exc("Invalid default model")
        if len(default_model) > 256:
            raise _sanitise_exc("Default model name too long")
        if any(ord(c) < 32 or ord(c) == 127 for c in default_model):
            raise _sanitise_exc("Default model contains control characters")

        providers = cfg.get("providers")
        if not isinstance(providers, list):
            raise _sanitise_exc("Invalid providers structure")
        if len(providers) > 256:
            raise _sanitise_exc("Too many providers")
        matches = [p for p in providers if isinstance(p, dict) and p.get("name") == default_provider]
        if len(matches) != 1:
            raise _sanitise_exc("Ambiguous or missing provider entry")
        chosen = matches[0]
        base_url = chosen.get("base_url")
        if not isinstance(base_url, str) or not base_url:
            raise _sanitise_exc("Provider base_url invalid")
        normalized = self._normalise_endpoint(base_url)
        endpoint_id = "sha256-" + _sha256_bytes(normalized.encode("ascii"))
        return {
            "provider_id": default_provider,
            "model_id": default_model,
            "endpoint_id": endpoint_id,
        }

    def _normalise_endpoint(self, raw: str) -> str:
        if not isinstance(raw, str):
            raise _sanitise_exc("Endpoint URL is not a string")
        try:
            raw_bytes = raw.encode("utf-8")
        except UnicodeEncodeError:
            raise _sanitise_exc("Malformed provider endpoint URL") from None
        if len(raw_bytes) > 4096:
            raise _sanitise_exc("Endpoint URL exceeds maximum length")
        if any(ord(c) < 32 or ord(c) == 127 for c in raw):
            raise _sanitise_exc("Endpoint URL contains control characters")
        if re.search(r'(redacted|<redacted>|key|token|secret|password)', raw, re.IGNORECASE):
            raise _sanitise_exc("Provider base_url contains placeholder")
        try:
            parsed = urllib.parse.urlparse(raw)
        except Exception:
            raise _sanitise_exc("Malformed provider endpoint URL") from None
        if parsed.scheme not in ("http", "https"):
            raise _sanitise_exc("Unsupported endpoint scheme")
        hostname = parsed.hostname
        if not hostname:
            raise _sanitise_exc("Missing hostname in endpoint")
        if any(ord(c) < 32 or ord(c) == 127 for c in hostname):
            raise _sanitise_exc("Hostname contains control characters")
        try:
            idna_host = hostname.encode("idna").decode("ascii").lower()
        except Exception:
            raise _sanitise_exc("Invalid international hostname") from None
        # validate labels
        labels = idna_host.split('.')
        if not labels or any(not label for label in labels):
            raise _sanitise_exc("Invalid hostname labels")
        for label in labels:
            if len(label) > 63:
                raise _sanitise_exc("Hostname label too long")
            if not re.match(r'^[a-z0-9]([a-z0-9-]*[a-z0-9])?$', label):
                raise _sanitise_exc("Invalid hostname label")
        scheme = parsed.scheme.lower()
        default_port = 80 if scheme == "http" else 443
        try:
            port = parsed.port
        except ValueError:
            raise _sanitise_exc("Invalid endpoint port") from None
        if port is not None and (not isinstance(port, int) or not 1 <= port <= 65535):
            raise _sanitise_exc("Invalid endpoint port")
        if port is None:
            port = default_port
        port_part = "" if port == default_port else f":{port}"
        if parsed.username or parsed.password:
            raise _sanitise_exc("Endpoint contains credentials")
        if parsed.query or parsed.fragment:
            raise _sanitise_exc("Endpoint must not have query or fragment")
        path = parsed.path or "/"
        # reject dot segments
        segments = path.split('/')
        for seg in segments:
            if seg in ('.', '..'):
                raise _sanitise_exc("Endpoint path contains dot segment")
        # reject malformed percent-encoding
        if re.search(r'%(?![0-9a-fA-F]{2})', path):
            raise _sanitise_exc("Malformed endpoint path encoding")
        # reject percent-encoded control characters
        try:
            unquoted = urllib.parse.unquote(path)
        except Exception:
            raise _sanitise_exc("Malformed endpoint path encoding") from None
        if any(ord(c) < 32 or ord(c) == 127 for c in unquoted):
            raise _sanitise_exc("Endpoint path contains control characters")
        if not path.endswith("/"):
            path += "/"
        return f"{scheme}://{idna_host}{port_part}{path}"

    def _systemctl_show(self, unit: str, user: bool = False) -> Dict[str, str]:
        argv = ["systemctl"]
        if user:
            argv.append("--user")
        argv.extend(["show", unit, "--property=MainPID", "--property=NRestarts",
                      "--property=ExecStart", "--property=FragmentPath"])
        res = self._run_cmd(argv, timeout=15, max_stdout=_MAX_SYSTEMCTL_OUT)
        if res.returncode != 0:
            raise _sanitise_exc("systemctl show failed")
        if not res.stdout:
            raise _sanitise_exc("Empty systemctl output")
        lines = res.stdout.decode("utf-8").splitlines()
        if not lines:
            raise _sanitise_exc("Empty systemctl output")
        seen: Dict[str, str] = {}
        allowed = {"MainPID", "NRestarts", "ExecStart", "FragmentPath"}
        for line in lines:
            if not line:
                raise _sanitise_exc("Malformed systemctl output line")
            if any(ord(c) < 32 or ord(c) == 127 for c in line):
                raise _sanitise_exc("Systemctl output contains control characters")
            if "=" not in line:
                raise _sanitise_exc("Malformed systemctl output line")
            key, value = line.split("=", 1)
            if key not in allowed:
                raise _sanitise_exc("Unexpected systemctl property")
            if key in seen:
                raise _sanitise_exc("Duplicate systemctl property")
            seen[key] = value
        for k in allowed:
            if k not in seen:
                raise _sanitise_exc("Missing systemctl property")
        return seen

    def _parse_execstart(self, raw: str) -> str:
        if any(ord(c) < 32 or ord(c) == 127 for c in raw):
            raise _sanitise_exc("ExecStart contains control characters")
        if len(raw) > 4096:
            raise _sanitise_exc("ExecStart value too long")
        if raw.startswith("{"):
            if not raw.endswith("}"):
                raise _sanitise_exc("Malformed ExecStart structure")
            inner = raw[1:-1]
            # count path= occurrences
            path_count = len(re.findall(r'\bpath=', inner))
            if path_count != 1:
                raise _sanitise_exc("ExecStart does not contain exactly one path= entry")
            m = re.search(r'\bpath=([^;}\s]*)', inner)
            if not m:
                raise _sanitise_exc("Could not parse ExecStart path")
            path = m.group(1)
        elif raw.startswith(_EXEC_PATH_LITERAL):
            parts = raw.split(None, 1)
            path = parts[0]
        else:
            raise _sanitise_exc("Unrecognised ExecStart format")
        if path != _EXEC_PATH_LITERAL:
            raise _sanitise_exc("ExecStart executable path is not /usr/bin/aletheon")
        return path

    def _check_fragment(self, fragment: str) -> None:
        if not os.path.isabs(fragment):
            raise _sanitise_exc("FragmentPath is not absolute")
        if any(ord(c) < 32 or ord(c) == 127 for c in fragment):
            raise _sanitise_exc("FragmentPath contains control characters")
        backing = self._mapper.fragment_to_backing(fragment)
        try:
            st = os.lstat(backing)
        except OSError:
            raise _sanitise_exc("FragmentPath backing does not exist or cannot be statted")
        if stat.S_ISLNK(st.st_mode):
            raise _sanitise_exc("FragmentPath backing is a symbolic link")
        if not stat.S_ISREG(st.st_mode):
            raise _sanitise_exc("FragmentPath backing is not a regular file")

    def _resolve_daemon_exe(self, pid: str, execstart_path: str) -> str:
        exe_link = self._proc_root / pid / "exe"
        try:
            target = os.readlink(os.fspath(exe_link))
        except OSError:
            return execstart_path
        if not target or os.path.isabs(target) is False:
            raise _sanitise_exc("proc exe symlink target is relative or empty")
        if target.endswith(" (deleted)"):
            raise _sanitise_exc("proc exe points to deleted binary")
        if len(target) > 4096:
            raise _sanitise_exc("proc exe target path too long")
        if any(ord(c) < 32 or ord(c) == 127 for c in target):
            raise _sanitise_exc("proc exe target contains control characters")
        logical = self._mapper.proc_exe_to_logical(target)
        if logical != _EXEC_PATH_LITERAL:
            raise _sanitise_exc("Resolved daemon executable is not /usr/bin/aletheon")
        return logical

    def _daemon_snapshot(self, unit: str, user: bool) -> Dict[str, Any]:
        props = self._systemctl_show(unit, user)
        pid_str = props["MainPID"]
        nrestarts_str = props["NRestarts"]
        if not _PID_RE.match(pid_str):
            raise _sanitise_exc("Invalid MainPID")
        if not _RESTARTS_RE.match(nrestarts_str):
            raise _sanitise_exc("Invalid NRestarts")
        pid = int(pid_str)
        nrestarts = int(nrestarts_str)
        execstart_path = self._parse_execstart(props["ExecStart"])
        self._check_fragment(props["FragmentPath"])
        self._resolve_daemon_exe(str(pid), execstart_path)
        return {"MainPID": pid, "NRestarts": nrestarts}

    def collect(self, fixture_digest: str, generation_id: str, repo_root: pathlib.Path) -> Dict[str, Any]:
        if not _validate_hex(fixture_digest, (64,)):
            raise _sanitise_exc("fixture_digest must be exactly 64 lowercase hex characters")
        if not _validate_generation_id(generation_id):
            raise _sanitise_exc(
                "generation_id must be nonempty ASCII letters/digits/dot/underscore/hyphen"
            )

        repo = self._repo_info(repo_root)
        build = self._build_info()
        artifact_sha = self._install_artifact()
        client_version, client_protov = self._version_info()
        provider = self._config_provider()

        snap1_m = self._daemon_snapshot("aletheon-core.service", user=False)
        snap1_u = self._daemon_snapshot("aletheon.service", user=True)
        snap1_mem = self._daemon_snapshot("aletheon-memory-agent.service", user=True)

        snap2_m = self._daemon_snapshot("aletheon-core.service", user=False)
        snap2_u = self._daemon_snapshot("aletheon.service", user=True)
        snap2_mem = self._daemon_snapshot("aletheon-memory-agent.service", user=True)

        if snap1_m != snap2_m:
            raise _sanitise_exc("Machine daemon state changed during collection")
        if snap1_u != snap2_u:
            raise _sanitise_exc("User daemon state changed during collection")
        if snap1_mem != snap2_mem:
            raise _sanitise_exc("Memory agent daemon state changed during collection")

        provenance: Dict[str, Any] = {
            "repo": repo,
            "build": build,
            "environment_level": "installed",
            "installed_artifact": {"path": _EXEC_PATH_LITERAL, "sha256": artifact_sha},
            "client": {"version": client_version, "protocol_version": client_protov},
            "daemons": {
                "machine": {
                    "path": _EXEC_PATH_LITERAL,
                    "sha256": artifact_sha,
                    "version": client_version,
                    "protocol_version": client_protov,
                },
                "user": {
                    "path": _EXEC_PATH_LITERAL,
                    "sha256": artifact_sha,
                    "version": client_version,
                    "protocol_version": client_protov,
                },
            },
            "provider": provider,
            "fixture_digest": fixture_digest,
            "generation_id": generation_id,
        }

        # Import contract validation
        try:
            from .acceptance_contract import validate_provenance
        except ImportError:
            try:
                from acceptance_contract import validate_provenance  # type: ignore[no-redef]
            except ImportError:
                try:
                    from harness.acceptance_contract import validate_provenance  # type: ignore[no-redef]
                except ImportError:
                    raise _sanitise_exc("acceptance_contract module not found") from None

        try:
            validated = validate_provenance(provenance)
        except Exception:
            raise _sanitise_exc("Provenance validation failed") from None
        return validated


class _SubprocessRunner:
    def run(self, argv: List[str], *, timeout: float, env: Optional[Dict[str, str]] = None) -> CommandResult:
        try:
            proc = subprocess.run(
                argv,
                timeout=timeout,
                capture_output=True,
                shell=False,
                env=env,
            )
        except subprocess.TimeoutExpired:
            raise _sanitise_exc("Command timed out") from None
        except FileNotFoundError:
            raise _sanitise_exc("Command not found") from None
        except PermissionError:
            raise _sanitise_exc("Permission denied for command") from None
        except OSError:
            raise _sanitise_exc("OS error running command") from None
        return CommandResult(returncode=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)


def collect_installed_provenance(
    repo_root: pathlib.Path,
    fixture_digest: str,
    generation_id: str,
    *,
    runner: Optional[CommandRunner] = None,
    installed_path: pathlib.Path = INSTALLED_PATH,
    proc_root: pathlib.Path = PROC_ROOT,
) -> Dict[str, Any]:
    if installed_path != INSTALLED_PATH:
        raise _sanitise_exc("installed_path must be /usr/bin/aletheon")
    if proc_root != PROC_ROOT:
        raise _sanitise_exc("proc_root must be /proc")
    if not _validate_hex(fixture_digest, (64,)):
        raise _sanitise_exc("fixture_digest must be exactly 64 lowercase hex characters")
    if not _validate_generation_id(generation_id):
        raise _sanitise_exc("generation_id must be nonempty ASCII letters/digits/dot/underscore/hyphen")

    if runner is None:
        runner = _SubprocessRunner()
    deps = _ProductionDependencies()
    collector = _Collector(
        runner=runner,
        installed_path=installed_path,
        proc_root=proc_root,
        install_checker=deps.install_checker,
        path_mapper=deps.path_mapper,
    )
    return collector.collect(fixture_digest, generation_id, repo_root)