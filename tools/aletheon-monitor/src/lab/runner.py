"""Compose one deterministic Nightwatch case and its sealed evidence."""

from __future__ import annotations

import os
import platform
import sys
import uuid
from pathlib import Path

from .config import expand_argument
from .diagnostics import diagnose
from .evidence import EvidenceBundle, atomic_write, utc_now
from .fingerprint import failure_fingerprint, signature_from_result
from .issue import render_issue_draft
from .model import CaseSpec, DiagnosticSettings
from .source import repository_facts, workspace_diff
from .store import LabStore
from .supervisor import run_bounded

_SAFE_INHERITED_ENVIRONMENT = frozenset(
    {
        "PATH",
        "LANG",
        "LC_ALL",
        "TZ",
        "USER",
        "LOGNAME",
        "HOME",
        "TMPDIR",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
        "RUSTUP_HOME",
        "CARGO_HOME",
        "CARGO_TARGET_DIR",
        "ALETHEON_CARGO_CACHE_ROOT",
        "ALETHEON_CARGO_TARGET_MAX_GIB",
        "CARGO_BUILD_JOBS",
    }
)


class CaseRunner:
    def __init__(
        self,
        *,
        state_root: Path,
        store: LabStore,
        diagnostics: DiagnosticSettings,
        max_capture_bytes: int,
    ):
        self.state_root = state_root
        self.store = store
        self.diagnostics = diagnostics
        self.max_capture_bytes = max_capture_bytes

    def _environment(
        self, case: CaseSpec, *, repo: Path, artifacts: Path, run_id: str
    ) -> dict[str, str]:
        environment = {
            key: value
            for key, value in os.environ.items()
            if key in _SAFE_INHERITED_ENVIRONMENT
        }
        for key, value in case.environment.items():
            if not key or "=" in key or "\0" in key:
                raise ValueError(f"invalid campaign environment key: {key!r}")
            environment[key] = expand_argument(
                value, repo=repo, artifacts=artifacts, run_id=run_id
            )
        return environment

    def run(
        self,
        case: CaseSpec,
        *,
        repository_root: Path,
        expected_sha: str,
        run_id: str | None = None,
    ) -> dict[str, object]:
        run_id = run_id or uuid.uuid4().hex
        artifacts_root = self.state_root / "artifacts"
        bundle = EvidenceBundle(artifacts_root, run_id)
        started_at = utc_now()
        facts = repository_facts(repository_root)
        command = tuple(
            expand_argument(
                value, repo=repository_root, artifacts=bundle.path, run_id=run_id
            )
            for value in case.command
        )
        manifest: dict[str, object] = {
            "schema_version": 1,
            "run_id": run_id,
            "case_id": case.case_id,
            "started_at": started_at,
            "source": {**facts, "expected_sha": expected_sha},
            "host": {
                "python": sys.version.split()[0],
                "platform": platform.platform(),
                "pid": os.getpid(),
            },
            "command": list(command),
            "timeout_seconds": case.timeout_seconds,
            "expected_exit_codes": list(case.expected_exit_codes),
        }
        bundle.write_json("manifest.json", manifest)
        bundle.append_event(
            "run_started", {"case_id": case.case_id, "commit_sha": facts["commit_sha"]}
        )

        invalid_reason = None
        if facts["commit_sha"] != expected_sha:
            invalid_reason = "worktree commit does not match scheduled SHA"
        elif case.require_clean and not facts["tree_clean"]:
            invalid_reason = "worktree was dirty before command execution"

        if invalid_reason is not None:
            process: dict[str, object] = {
                "argv": list(command),
                "executed": False,
                "exit_code": None,
                "timed_out": False,
                "spawn_error": None,
                "process_group_reaped": False,
                "stdout_preview": "",
                "stderr_preview": invalid_reason,
            }
            outcome = "invalid"
            failure_class = "configuration_failure"
            invariants = ["pinned_clean_source_required", "command_must_execute"]
        else:
            environment = self._environment(
                case, repo=repository_root, artifacts=bundle.path, run_id=run_id
            )
            bundle.append_event("process_spawning", {"argv": list(command)})
            process = run_bounded(
                command,
                cwd=repository_root,
                environment=environment,
                timeout_seconds=case.timeout_seconds,
                stdout_path=bundle.path / "stdout.log",
                stderr_path=bundle.path / "stderr.log",
                max_capture_bytes=self.max_capture_bytes,
            )
            process["executed"] = True
            bundle.append_event(
                "process_finished",
                {
                    "exit_code": process["exit_code"],
                    "timed_out": process["timed_out"],
                    "process_group_reaped": process["process_group_reaped"],
                },
            )
            if not process["process_group_reaped"]:
                outcome = "invalid"
                failure_class = "cleanup_failure"
                invariants = ["process_group_must_be_reaped"]
            elif process["spawn_error"]:
                outcome = "infra_blocked"
                failure_class = "spawn_failure"
                invariants = ["command_must_execute"]
            elif process["timed_out"]:
                outcome = "product_failed"
                failure_class = "timeout"
                invariants = ["case_must_reach_authoritative_terminal"]
            elif process["exit_code"] not in case.expected_exit_codes:
                outcome = "product_failed"
                failure_class = "command_failure"
                invariants = ["deterministic_command_must_pass"]
            else:
                outcome = "passed"
                failure_class = "none"
                invariants = []

        workspace_diff(repository_root, bundle.path / "workspace.diff")
        finished_at = utc_now()
        result: dict[str, object] = {
            "schema_version": 1,
            "run_id": run_id,
            "case_id": case.case_id,
            "started_at": started_at,
            "finished_at": finished_at,
            "outcome": outcome,
            "failure_class": failure_class,
            "source": facts,
            "oracle": {
                "executed": bool(process.get("executed")),
                "complete": bool(process.get("executed"))
                and bool(process.get("process_group_reaped")),
                "passed": outcome == "passed",
                "invariant_ids": invariants,
            },
            "process": process,
        }
        if outcome != "passed":
            result["fingerprint"] = failure_fingerprint(
                case_id=case.case_id,
                failure_class=failure_class,
                signature=signature_from_result(result),
                invariant_ids=invariants,
            )
        bundle.write_json("result.json", result)
        analysis = None
        if outcome != "passed" and case.diagnose_on_failure:
            has_credential = bool(os.environ.get(self.diagnostics.api_key_env))
            budget_available = (
                not self.diagnostics.enabled
                or not has_credential
                or self.store.reserve_diagnostic_call(
                    run_id=run_id,
                    requested_at=finished_at,
                    max_calls_per_day=self.diagnostics.max_calls_per_day,
                )
            )
            if budget_available:
                analysis = diagnose(self.diagnostics, result)
            else:
                analysis = {
                    "schema_version": 1,
                    "status": "budget_exhausted",
                    "error": "daily diagnostic call budget exhausted",
                }
            bundle.write_json("analysis.json", analysis)
            bundle.append_event("diagnosis_finished", {"status": analysis["status"]})
        bundle.append_event("bundle_sealing", {"outcome": outcome})
        bundle.seal()

        cluster = self.store.record(result, bundle.path)
        if outcome != "passed":
            draft = render_issue_draft(result, cluster, analysis)
            drafts = self.state_root / "issue-drafts"
            drafts.mkdir(parents=True, exist_ok=True, mode=0o700)
            fingerprint_name = str(result["fingerprint"]).removeprefix("sha256:")
            atomic_write(drafts / f"{fingerprint_name}.md", draft.encode())
        result["cluster"] = cluster
        result["bundle_path"] = str(bundle.path)
        return result
