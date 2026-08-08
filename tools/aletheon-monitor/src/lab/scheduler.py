"""Sequential 24x7 scheduler with pinned-source worktrees."""

from __future__ import annotations

import json
import os
import signal
import time
import uuid
from datetime import datetime, timezone
from typing import TextIO

from .model import LabSettings
from .runner import CaseRunner
from .source import SourceManager
from .store import LabStore


def _parse_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


class Nightwatch:
    def __init__(self, settings: LabSettings, *, output: TextIO):
        self.settings = settings
        self.output = output
        self.stop_requested = False
        self.last_cycle_error_count = 0
        self.settings.state_root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.store = LabStore(self.settings.state_root / "state" / "lab.sqlite")
        self.source = SourceManager(
            self.settings.state_root,
            self.settings.source.repository_url,
            self.settings.source.ref,
        )
        self.runner = CaseRunner(
            state_root=self.settings.state_root,
            store=self.store,
            diagnostics=self.settings.diagnostics,
            max_capture_bytes=self.settings.max_capture_bytes,
        )

    def close(self) -> None:
        self.store.close()

    def request_stop(self, *_: object) -> None:
        self.stop_requested = True

    def _emit(self, event: str, **fields: object) -> None:
        record = json.dumps(
            {"event": event, **fields}, sort_keys=True, ensure_ascii=False
        )
        event_path = self.settings.state_root / "scheduler-events.jsonl"
        descriptor = os.open(event_path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(descriptor, (record + "\n").encode())
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        print(
            record,
            file=self.output,
            flush=True,
        )

    def _due(self, case_id: str, interval_seconds: int) -> bool:
        latest = self.store.latest_finished_at(case_id)
        if latest is None:
            return True
        elapsed = datetime.now(timezone.utc) - _parse_timestamp(latest)
        return elapsed.total_seconds() >= interval_seconds

    def run_case(self, case_id: str) -> dict[str, object]:
        case = next(
            (item for item in self.settings.cases if item.case_id == case_id), None
        )
        if case is None:
            raise KeyError(f"unknown case_id: {case_id}")
        commit = self.source.sync()
        run_id = uuid.uuid4().hex
        worktree = self.source.create_worktree(run_id, commit)
        try:
            result = self.runner.run(
                case,
                repository_root=worktree,
                expected_sha=commit,
                run_id=run_id,
            )
        finally:
            try:
                self.source.remove_worktree(worktree)
            except Exception as error:
                self._emit(
                    "worktree_cleanup_failed",
                    run_id=run_id,
                    path=str(worktree),
                    error=f"{type(error).__name__}: {error}",
                )
                raise
        self._emit(
            "case_finished",
            run_id=result["run_id"],
            case_id=result["case_id"],
            outcome=result["outcome"],
            fingerprint=result.get("fingerprint"),
        )
        return result

    def cycle(self, *, force: bool = False) -> list[dict[str, object]]:
        results = []
        self.last_cycle_error_count = 0
        for case in self.settings.cases:
            if self.stop_requested:
                break
            if not case.enabled:
                continue
            if force or self._due(case.case_id, case.interval_seconds):
                try:
                    results.append(self.run_case(case.case_id))
                # A single malformed checkout or campaign must not stop the
                # long-running supervisor. The error remains durable below.
                except Exception as error:  # noqa: BLE001
                    self.last_cycle_error_count += 1
                    self._emit(
                        "scheduler_error",
                        case_id=case.case_id,
                        error=f"{type(error).__name__}: {error}",
                    )
        return results

    def watch(self, *, max_cycles: int | None = None) -> None:
        previous_handlers = {}
        for requested_signal in (signal.SIGINT, signal.SIGTERM):
            previous_handlers[requested_signal] = signal.signal(
                requested_signal, self.request_stop
            )
        cycles = 0
        try:
            while not self.stop_requested:
                self.cycle()
                cycles += 1
                if max_cycles is not None and cycles >= max_cycles:
                    break
                deadline = time.monotonic() + self.settings.poll_seconds
                while not self.stop_requested and time.monotonic() < deadline:
                    time.sleep(min(0.5, deadline - time.monotonic()))
        finally:
            for requested_signal, handler in previous_handlers.items():
                signal.signal(requested_signal, handler)
